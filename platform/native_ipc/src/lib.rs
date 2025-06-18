//! # Tooling IPC (Inter-Process Communication)
//!
//! Creates a WebSocket which allows an external process to send
//! commands/requests for the game platform.

//! Tooling IPC (Inter-Process Communication)
//!
//! Creates a WebSocket which allows an external process to send
//! commands/requests for the game platform.
//!
//! The initial usage is for the remote Editor to load/unload modules. Over
//! time, the usage is expected to broaden, thus the more generic name "Tools
//! IPC".

use flatbuffers::FlatBufferBuilder;
use libloading::Library;
use native_common::{GameEngine, ecs_module::EcsModuleDynamic};

/// Messages sent from the remote client to this code.
pub mod tooling_messages {
    #![allow(clippy::all, clippy::pedantic, warnings, unused, unused_imports)]
    include!(concat!(env!("OUT_DIR"), "/tooling_messages_generated.rs"));
}
use tooling_messages::IpcToHostT;

/// Internal (private) handlers for messages from the client.
///
/// If you're looking to add functionality to the Tooling IPC API, this is
/// likely the place to do it.
fn handle_request(engine: &mut GameEngine, message: &IpcToHostT) -> Option<Vec<u8>> {
    match &message {
        IpcToHostT::LoadModule(message) => {
            let module_path = std::path::PathBuf::from(&message.path.as_ref().unwrap());
            log::info!("load module {}", module_path.display());

            let library = unsafe { Library::new(module_path).unwrap() };
            let module = Box::new(unsafe { EcsModuleDynamic::new(library) });
            engine.register_ecs_module(module);
        }
        IpcToHostT::UnloadModule(message) => {
            let module_path = std::path::PathBuf::from(&message.path.as_ref().unwrap());
            log::info!("unload module {}", module_path.display());
        }
        IpcToHostT::ReloadModule(message) => {
            let module_path = std::path::PathBuf::from(&message.path.as_ref().unwrap());
            log::info!("reload module {}", module_path.display());
        }
        IpcToHostT::ListModules(_) => {
            return Some(modules_flat_buffer(engine));
        }
        IpcToHostT::ListSystems(_) => {
            return Some(systems_flat_buffer(engine));
        }
        IpcToHostT::NONE | IpcToHostT::EnginePause(_) => unreachable!(),
    }
    None
}

/// A list of game modules as a flat buffer.
fn modules_flat_buffer(engine: &mut GameEngine) -> Vec<u8> {
    let module_names = engine.esc_module_names();

    let mut builder = FlatBufferBuilder::new();
    let mut entries = Vec::new();
    for name in module_names {
        let name = Some(builder.create_string(name));
        let path = Some(builder.create_string("path/cats"));
        let module = tooling_messages::Module::create(
            &mut builder,
            &tooling_messages::ModuleArgs { name, path },
        );
        entries.push(module);
    }
    let modules = builder.create_vector(&entries);

    let offset = tooling_messages::Modules::create(
        &mut builder,
        &tooling_messages::ModulesArgs {
            list: Some(modules),
        },
    );
    let offset = tooling_messages::MessageToClient::create(
        &mut builder,
        &tooling_messages::MessageToClientArgs {
            message_type: tooling_messages::IpcToClient::Modules,
            message: Some(offset.as_union_value()),
        },
    );
    builder.finish_minimal(offset);
    builder.finished_data().to_vec()
}

/// A list of systems as a flat buffer.
fn systems_flat_buffer(engine: &mut GameEngine) -> Vec<u8> {
    let systems = engine.system_names();

    let mut builder = FlatBufferBuilder::new();
    let mut entries = Vec::new();
    for name in systems {
        let name = Some(builder.create_string(name));
        let system = tooling_messages::System::create(
            &mut builder,
            &tooling_messages::SystemArgs {
                name,
                enabled: true,
            },
        );
        entries.push(system);
    }
    let systems = builder.create_vector(&entries);

    let offset = tooling_messages::Systems::create(
        &mut builder,
        &tooling_messages::SystemsArgs {
            list: Some(systems),
        },
    );
    let offset = tooling_messages::MessageToClient::create(
        &mut builder,
        &tooling_messages::MessageToClientArgs {
            message_type: tooling_messages::IpcToClient::Systems,
            message: Some(offset.as_union_value()),
        },
    );
    builder.finish_minimal(offset);
    builder.finished_data().to_vec()
}

pub mod tooling {
    use core::cmp::PartialEq;
    use std::{
        fmt::{self, Debug},
        io::{self, ErrorKind, Read, Write},
        net::TcpListener,
        sync::mpsc::{self, Receiver, Sender, TryRecvError},
        thread::{JoinHandle, spawn, yield_now},
    };

    use tungstenite::{Error, accept, protocol::WebSocket};

    use super::*;

    /// Message types which are sent from the IPC threads to the main thread.
    #[derive(Debug, PartialEq)]
    enum ToHost {
        /// Unable to listen on that port. No results (e.g. `Opened`, `Message`)
        /// will be generated for this port.
        ListenFailed { port: u16 },

        /// A new channel thread was created for a new connection.
        Opened(IpcConnection),

        /// A data message from the remote connection.
        Message { content: Vec<u8> },

        /// The connection is closed.
        Closed,
    }

    /// Message types which are sent from the main thread to the listener thread.
    #[derive(Debug, PartialEq)]
    enum ToListener {}

    /// Information sent from the IPC Host Module to an open WebSocket connection
    /// thread.
    #[derive(Debug, PartialEq)]
    pub(crate) enum ToClient {
        Message(Vec<u8>),
    }

    /// Thread function for listening for new WebSocket connections.
    fn ipc_listener_thread(port: u16, to_host: &Sender<ToHost>, from_host: &Receiver<ToListener>) {
        let Ok(server) = TcpListener::bind(("127.0.0.1", port)) else {
            let _ = to_host.send(ToHost::ListenFailed { port });
            return;
        };
        server
            .set_nonblocking(true)
            .expect("Setting server to non-blocking.");
        for stream_result in server.incoming() {
            if let Ok(stream) = stream_result {
                // Set the socket to blocking for the websocket handshake.
                stream.set_nonblocking(false).unwrap();
                let mut websocket = accept(stream).unwrap();
                websocket.get_mut().set_nonblocking(true).unwrap();
                let record = IpcConnection::new(&to_host.clone(), websocket);
                to_host.send(ToHost::Opened(record)).unwrap();
            }
            match from_host.try_recv() {
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) | Ok(_) => (),
            }
            yield_now();
        }
        log::info!("listener thread exit");
    }

    /// Thread function for an open WebSocket connection.
    ///
    /// Disconnecting the `from_host` channel will cause the thread to terminate.
    fn ipc_connection_thread<T>(
        mut websocket: WebSocket<T>,
        to_host: &Sender<ToHost>,
        from_host: &Receiver<ToClient>,
    ) where
        T: Send + Read + Write + 'static,
    {
        loop {
            match from_host.try_recv() {
                Err(TryRecvError::Disconnected) => {
                    log::info!("tooling_ipc from_host TryRecvError::Disconnected");
                    break;
                }
                Err(TryRecvError::Empty) => (),
                Ok(ToClient::Message(message)) => {
                    websocket.send(message.into()).unwrap();
                }
            }
            match websocket.read() {
                Ok(m) => {
                    if m.is_binary() {
                        let content = m.into_data().into();
                        log::info!("websocket read into_data {:?}", content);
                        let _ = to_host.send(ToHost::Message { content });
                    } else if m.is_text() {
                        // If text is required, please file a feature request add support.
                        log::trace!("IPC Host: Not expecting text IPC {:?}", m.into_text());
                    }
                }
                Err(Error::Io(e)) => match e.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => (),
                    _ => {
                        // When this is reported, work out how the issue should
                        // handled, such as forwarding to the IPC Agent or ignoring
                        // it here (maybe close the connection).
                        panic!("IPC Host: Please report this bug. Error::Io {:?}", e);
                    }
                },
                Err(Error::ConnectionClosed) => {
                    let _ = to_host.send(ToHost::Closed);
                    log::info!("tooling_ipc websocket.read Error::ConnectionClosed");
                    break;
                }
                Err(e) => {
                    // When this is reported, work out how the issue should handled,
                    // such as forwarding to the IPC Agent or ignoring it here
                    // (maybe close the connection).
                    panic!("IPC Host: Please report this bug. Error {:?}", e);
                }
            }
            yield_now();
        }
        log::info!("tooling ipc connection thread exit");
    }

    /// An open WebSocket connection.
    pub(crate) struct IpcConnection {
        /// This record may own a handle to a background thread which allows
        /// 'joining' the thread to be certain it has terminated.
        thread: Option<JoinHandle<()>>,

        /// Endpoint of a communication channel to send messages to the background
        /// connection thread.
        ///
        /// Closing (i.e. dropping) the channel will let the thread know it's time
        /// to terminate.
        to_client: Option<Sender<ToClient>>,
    }

    impl PartialEq for IpcConnection {
        /// Here, `Option` fields are considered 'equal' if values are equally
        /// `Some` or `None`. Internal values of `Some` are not compared.
        fn eq(&self, other: &Self) -> bool {
            self.thread.is_none() == other.thread.is_none()
                && self.to_client.is_none() == other.to_client.is_none()
        }
    }

    impl Debug for IpcConnection {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("IpcConnection")
                .field("thread", &self.thread.is_some())
                .field("to_client", &self.to_client.is_some())
                .finish()
        }
    }

    impl IpcConnection {
        fn new<T>(to_host: &Sender<ToHost>, websocket: WebSocket<T>) -> Self
        where
            T: Send + Read + Write + 'static,
        {
            let to_host_endpoint = to_host.clone();
            let (to_client, from_host) = mpsc::channel::<ToClient>();
            let thread = spawn(move || {
                ipc_connection_thread(websocket, &to_host_endpoint, &from_host);
            });
            Self {
                thread: Some(thread),
                to_client: Some(to_client),
            }
        }

        /// Send a message to the remote end of the connection.
        pub fn to_client(&self) -> &Sender<ToClient> {
            self.to_client.as_ref().unwrap()
        }
    }

    impl Drop for IpcConnection {
        fn drop(&mut self) {
            // When the channel is disconnected by dropping our end, the
            // listening thread will close the port and terminate.
            if let Some(endpoint) = self.to_client.take() {
                drop(endpoint);
            }

            // The thread is only expected to exit (be joinable) if the to_client
            // was closed (is none). If this assert fails, join would block
            // indefinitely. The assert is expected to be easier to debug.
            assert!(self.to_client.is_none());
            let _ = self.thread.take().unwrap().join();
        }
    }

    /// State of the IPC server for tooling.
    ///
    /// Each instance will listen on a given port for WebSocket connections (give
    /// each a unique port number), though creating more than one instance is not
    /// expected.
    pub struct ToolingIpc {
        /// This record may own a handle to a background thread which allows
        /// 'joining' the thread to be certain it has terminated.
        thread: Option<JoinHandle<()>>,

        /// If a WebSocket connection is open, `client` will be `Some()`.
        client: Option<IpcConnection>,

        /// Endpoint of a communication channel to send messages to the background
        /// listener thread.
        ///
        /// Closing (i.e. dropping) the channel will let the thread know it's time
        /// to terminate.
        to_listener: Option<Sender<ToListener>>,

        /// Messages from listener OR open connection threads.
        from_threads: Option<Receiver<ToHost>>,

        /// Control whether the main game loop should execute `engine::frame()`.
        should_run_frame: bool,
    }

    impl ToolingIpc {
        /// Create a new `ToolingIpc` listening on a the given `port`.
        pub fn on_port(port: u16) -> io::Result<Self> {
            log::info!("ToolingIpc on_port({port})");

            let (to_host, from_threads) = mpsc::channel::<ToHost>();
            let (to_listener, from_host) = mpsc::channel::<ToListener>();
            let thread = spawn(move || {
                ipc_listener_thread(port, &to_host, &from_host);
            });
            let client = None;
            Ok(Self {
                thread: Some(thread),
                client,
                to_listener: Some(to_listener),
                from_threads: Some(from_threads),
                should_run_frame: true,
            })
        }

        /// Run just before the frame runs.
        ///
        /// Return true to run the `engine::frame()` as normal, false to prevent
        /// `engine::frame()` execution (ask again for the next frame).
        pub fn process_ipc_messages(&mut self, engine: &mut GameEngine) -> bool {
            if let Some(from_threads) = &self.from_threads {
                loop {
                    match from_threads.try_recv() {
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            log::info!("connection broken");
                            break;
                        }
                        Ok(msg) => match msg {
                            ToHost::Closed => log::info!("ToHost::Closed"),
                            ToHost::ListenFailed { port } => {
                                log::info!("ToHost::ListenFailed {port}");
                            }
                            ToHost::Message { content } => {
                                log::info!("ToHost::Message content {:?}", content);
                                let msg = flatbuffers::root::<tooling_messages::MessageToHost<'_>>(
                                    &content,
                                )
                                .unwrap();
                                match &msg.unpack().message {
                                    tooling_messages::IpcToHostT::NONE => {}
                                    tooling_messages::IpcToHostT::EnginePause(message) => {
                                        self.should_run_frame = !message.paused;
                                        println!("should_run_frame {}", self.should_run_frame);
                                    }
                                    _ => {
                                        if let Some(response) =
                                            handle_request(engine, &msg.unpack().message)
                                        {
                                            if let Some(connection) = &self.client {
                                                connection
                                                    .to_client()
                                                    .send(ToClient::Message(response))
                                                    .unwrap();
                                            }
                                        }
                                    }
                                }
                            }
                            ToHost::Opened(connection) => {
                                log::info!(
                                    "ToHost::Opened thread {}, open {}",
                                    connection.thread.is_some(),
                                    connection.to_client.is_some()
                                );
                                self.client = Some(connection);
                            }
                        },
                    }
                }
            }
            self.should_run_frame
        }
    }

    /// The thread(s) created by the `ToolingIpc` need to be cleaned up (i.e.
    /// returning limited resources).
    ///
    /// The listener thread is explicitly joined here, the connection threads are
    /// cleaned up in the drop implementation for `self.client` (i.e.
    /// `IpcConnection`).
    impl Drop for ToolingIpc {
        fn drop(&mut self) {
            // When the channel is disconnected by dropping our end, the
            // listening thread will close the port and terminate.
            if let Some(endpoint) = self.to_listener.take() {
                drop(endpoint);
            }
            let _ = self.thread.take().unwrap().join();
        }
    }
}
