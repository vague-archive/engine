//! WebSocket threads and data types for Inter Process Communication (IPC).
//!
//! See [`./lib.rs`] and [`../README.md`] for documentation.

// Required for `EventReader` and `EventWriter`.
#![allow(clippy::needless_pass_by_value)]

use std::{
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread::{spawn, yield_now, JoinHandle},
};

use tungstenite::{accept, protocol::WebSocket, Error};

/// A TCP/IP port number.
pub(crate) type Port = u16;

/// An internal number to uniquely identify a connection.
pub(crate) type Channel = u16;

/// Messages sent from background threads to the IPC Host Module (system).
#[derive(Debug, PartialEq)]
pub(crate) enum ToHostSystem {
    /// Unable to listen on that port. No results (e.g. `Opened`, `Message`)
    /// will be generated for this port (until a request to listen on the port
    /// succeeds).
    ListenFailed { port: Port, user_id: String },

    /// No more new connections (`Opened` results) will be generated for this
    /// port (until another request to listen on that port succeeds).
    PortIgnored { port: Port },

    /// A new channel thread was created for a new connection.
    Opened(IpcChannelRecord),

    /// A data message from the remote connection.
    Message { channel: Channel, content: Vec<u8> },

    /// The connection to the remote is closed, no more data messages may be
    /// sent.
    Closed { channel: Channel },
}

/// Information sent from the IPC Host Module to a specific channel thread.
#[derive(Debug, PartialEq)]
pub(crate) enum ToChannel {
    Message(Vec<u8>),
}

/// Information sent from the IPC Host Module to the listener thread for a specific port.
pub(crate) enum ToListener {}

/// A port listener as seen from the IPC Host Module.
pub(crate) struct IpcListenerRecord {
    /// Identify where the request to create this listener came from.
    // TODO(https://github.com/vaguevoid/engine/issues/320): Remove user_id.
    user_id: String,

    /// This record may own a handle to a background thread which allows
    /// 'joining' the thread to be certain it has terminated.
    thread: Option<JoinHandle<()>>,

    /// Endpoint of a communication channel to send messages to the background
    /// listener thread.
    ///
    /// Closing (i.e. dropping) the channel will let the thread know it's time
    /// to terminate.
    to_listener: Option<Sender<ToListener>>,
}

impl IpcListenerRecord {
    pub fn new(port: Port, user_id: &str, sender: Sender<ToHostSystem>) -> Self {
        let user_id_name = user_id.to_string();
        let (to_listener, from_module) = mpsc::channel::<ToListener>();
        // This is exempt from the 'no spawning threads in modules' rule
        // because this is an extension.
        let thread = spawn(move || {
            ipc_listener_thread(port, user_id_name, sender, from_module);
        });
        Self {
            user_id: user_id.to_string(),
            thread: Some(thread),
            to_listener: Some(to_listener),
        }
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }
}

impl Drop for IpcListenerRecord {
    fn drop(&mut self) {
        // When the channel is disconnected by dropping our end, the
        // listening thread will close the port and terminate.
        drop(self.to_listener.take().unwrap());
        let _ = self.thread.take().unwrap().join();
    }
}

/// A WebSocket connection as seen from the IPC Host Module.
pub(crate) struct IpcChannelRecord {
    /// The TCP/IP port number where the connection was initially made.
    port: Port,

    /// A unique identifier used to route messages to the right endpoints.
    ///
    /// This value is currently implemented as the port the connection is on,
    /// but any unique value will do. (Avoid reading the value as a port number,
    /// that is subject to change without notice).
    channel: Channel,

    /// Identifier for the request used to listen for connections.
    // TODO(https://github.com/vaguevoid/engine/issues/320): Remove user_id.
    user_id: String,

    /// This record may own a handle to a background thread which allows
    /// 'joining' the thread to be certain it has terminated.
    thread: Option<JoinHandle<()>>,

    /// Endpoint of a communication channel to send messages to the background
    /// connection thread.
    ///
    /// Closing (i.e. dropping) the channel will let the thread know it's time
    /// to terminate.
    to_channel: Option<Sender<ToChannel>>,
}
use core::cmp::PartialEq;

impl PartialEq for IpcChannelRecord {
    /// Here, `Option` fields are considered 'equal' if values are equally
    /// `Some` or `None`. Internal values of `Some` are not compared.
    fn eq(&self, other: &Self) -> bool {
        self.port == other.port
            && self.channel == other.channel
            && self.user_id == other.user_id
            && self.thread.is_none() == other.thread.is_none()
            && self.to_channel.is_none() == other.to_channel.is_none()
    }
}

use std::fmt::{self, Debug};

impl Debug for IpcChannelRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IpcChannelRecord")
            .field("port", &self.port)
            .field("channel", &self.channel)
            .field("user_id", &self.user_id)
            .field("thread", &self.thread.is_some())
            .field("to_channel", &self.to_channel.is_some())
            .finish()
    }
}

impl IpcChannelRecord {
    pub fn new<T>(
        port: Port,
        channel: Channel,
        user_id: &str,
        sender: Sender<ToHostSystem>,
        websocket: WebSocket<T>,
    ) -> Self
    where
        T: Send + Read + Write + 'static,
    {
        // let module_endpoint = to_host_system.clone();
        let (to_channel, from_module) = mpsc::channel::<ToChannel>();
        let thread = spawn(move || {
            ipc_channel_thread(channel, websocket, sender, from_module);
        });
        Self {
            port,
            channel,
            user_id: user_id.to_string(),
            thread: Some(thread),
            to_channel: Some(to_channel),
        }
    }
    pub fn channel(&self) -> Channel {
        self.channel
    }
    pub fn port(&self) -> Port {
        self.port
    }
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    /// Send a message to the remote end of the connection.
    pub fn to_remote(&self) -> &Sender<ToChannel> {
        self.to_channel.as_ref().unwrap()
    }
}

impl Drop for IpcChannelRecord {
    fn drop(&mut self) {
        // When the channel is disconnected by dropping our end, the
        // listening thread will close the port and terminate.
        if let Some(endpoint) = self.to_channel.take() {
            drop(endpoint);
        }

        // The thread is only expected to exit (be joinable) if the to_channel
        // was closed (is none). If this assert fails, join would block
        // indefinitely. The assert is expected to be easier to debug.
        assert!(self.to_channel.is_none());
        let _ = self.thread.take().unwrap().join();
    }
}

/// Called once for each port being listened to on a thread.
///
/// When the game is no longer interested in listening on this port,
/// disconnecting the `from_module` channel will cause the thread to terminate.
pub(crate) fn ipc_listener_thread(
    port: Port,
    user_id: String,
    to_host_system: Sender<ToHostSystem>,
    from_module: Receiver<ToListener>,
) {
    let user_id = user_id.clone();

    let Ok(server) = TcpListener::bind(("127.0.0.1", port)) else {
        let _ = to_host_system.send(ToHostSystem::ListenFailed { port, user_id });
        return;
    };
    server
        .set_nonblocking(true)
        .expect("Setting server to non-blocking.");
    for stream_result in server.incoming() {
        if let Ok(stream) = stream_result {
            // This is a local connection, so the peer address port will be unique.
            let channel = stream.peer_addr().unwrap().port();

            // Set the socket to blocking for the websocket handshake.
            stream.set_nonblocking(false).unwrap();
            let mut websocket = accept(stream).unwrap();
            websocket.get_mut().set_nonblocking(true).unwrap();
            let record =
                IpcChannelRecord::new(port, channel, &user_id, to_host_system.clone(), websocket);
            let _ = to_host_system.send(ToHostSystem::Opened(record));
        }
        match from_module.try_recv() {
            Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) | Ok(_) => (),
        }
        yield_now();
    }
    let _ = to_host_system.send(ToHostSystem::PortIgnored { port });
}

/// Called once for each active connection (channel) on a thread.
///
/// When the game is no longer interested in this channel, disconnecting the
/// `from_module` channel will cause the thread to terminate.
pub(crate) fn ipc_channel_thread<T>(
    channel: Channel,
    mut websocket: WebSocket<T>,
    to_host_system: Sender<ToHostSystem>,
    from_module: Receiver<ToChannel>,
) where
    T: Send + Read + Write + 'static,
{
    log::trace!("IPC Host: enter channel thread {channel}");
    loop {
        match from_module.try_recv() {
            Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => (),
            Ok(ToChannel::Message(message)) => {
                log::trace!(
                    "IPC Host: message for channel {channel} from agent {:?}",
                    message
                );
                websocket.send(message.into()).unwrap();
            }
        }
        match websocket.read() {
            Ok(m) => {
                if m.is_binary() {
                    let content = m.into_data().into();
                    let _ = to_host_system.send(ToHostSystem::Message { channel, content });
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
                let _ = to_host_system.send(ToHostSystem::Closed { channel });
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
    log::trace!("IPC Host: exiting channel thread {channel}");
}

#[cfg(test)]
mod tests {
    use std::{io, io::Cursor};

    use tungstenite::protocol::Role;

    use super::*;

    struct WriteMoc<Stream>(Stream);

    impl<Stream> io::Write for WriteMoc<Stream> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<Stream: io::Read> io::Read for WriteMoc<Stream> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    #[test]
    fn test_ipc_listener_record_drop() {
        let port = 9999;
        let user_id = "test_user_id";
        let (to_host_system, from_ipc_threads) = mpsc::channel::<ToHostSystem>();

        // Creating a scope to test with (Calling `drop` would work, but this
        // illustrates the common case of exiting a scope).
        {
            let subject = IpcListenerRecord::new(port, user_id, to_host_system);

            assert!(subject.thread.is_some());

            // Before the `drop` the error returned will be `Empty`. I.e. the
            // thread is there, but there are no messages.
            let result = from_ipc_threads.try_recv();
            assert_eq!(result.err().unwrap(), TryRecvError::Empty);
        }
        // The `drop` just happened.

        // The listener will put one last message into the channel, saying that
        // this port no longer has a listener.
        let result: Result<ToHostSystem, TryRecvError> = from_ipc_threads.try_recv();
        assert_eq!(result, Ok(ToHostSystem::PortIgnored { port }));

        // After that, the error returned will be `Disconnected`.
        let result: Result<ToHostSystem, TryRecvError> = from_ipc_threads.try_recv();
        assert_eq!(result.err().unwrap(), TryRecvError::Disconnected);
    }

    #[test]
    fn test_ipc_channel_record_drop() {
        let port = 9999;
        let channel = 1234;
        let user_id = "test_user_id";
        let (to_host_system, from_ipc_threads) = mpsc::channel::<ToHostSystem>();

        let incoming = Cursor::new(vec![]);
        let websocket = WebSocket::from_raw_socket(WriteMoc(incoming), Role::Client, None);

        {
            let subject = IpcChannelRecord::new(port, channel, user_id, to_host_system, websocket);

            assert!(subject.thread.is_some());

            // Before the `drop` the error returned will be `Empty`. I.e. the
            // thread is there, but there are no messages.
            let result = from_ipc_threads.try_recv();
            assert_eq!(result.err().unwrap(), TryRecvError::Empty);
        }
        // The `drop` just happened.

        // After the `drop` the error returned will be `Disconnected`. I.e. the
        // channel is gone and the thread is joined (terminated).
        let result: Result<ToHostSystem, TryRecvError> = from_ipc_threads.try_recv();
        assert_eq!(result.err().unwrap(), TryRecvError::Disconnected);
    }

    #[test]
    fn test_ipc_channel_record_messages() {
        let port = 5678;
        let channel = 3333;
        let user_id = "test_messaging_user_id";
        let (to_host_system, from_ipc_threads) = mpsc::channel::<ToHostSystem>();

        const BINARY_MESSAGE: u8 = 0x82;
        let incoming = Cursor::new(vec![
            // Some binary bytes.
            BINARY_MESSAGE,
            0x02,
            0x01,
            0x02,
            // There is a text message type, but text also works in binary.
            BINARY_MESSAGE,
            0x03,
            0x61,
            0x62,
            0x63,
        ]);
        let websocket = WebSocket::from_raw_socket(WriteMoc(incoming), Role::Client, None);

        let subject = IpcChannelRecord::new(port, channel, user_id, to_host_system, websocket);

        assert_eq!(subject.port(), port);
        assert_eq!(subject.channel(), channel);
        assert_eq!(subject.user_id(), user_id);
        assert!(subject.thread.is_some());
        assert!(subject.to_channel.is_some());

        let result = from_ipc_threads.recv();
        assert_eq!(
            result.ok().unwrap(),
            ToHostSystem::Message {
                channel,
                content: vec![0x01, 0x2]
            }
        );

        let result = from_ipc_threads.recv();
        assert_eq!(
            result.ok().unwrap(),
            ToHostSystem::Message {
                channel,
                content: vec![0x61, 0x62, 0x63]
            }
        );
    }
}
