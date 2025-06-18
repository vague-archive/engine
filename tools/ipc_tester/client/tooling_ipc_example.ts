//! Example of connecting to a Fiasco IPC Host.
//!
//! See [`../README.md`] for documentation.

import * as flatbuffers from 'flatbuffers';

/// Message send from this code to the game engine.
import { LoadModule, IpcToHost, MessageToHost, ListModules, ListSystems, EnginePause } from './gen/tooling_messages.ts';

/// Messages send from the game engine to this code.
import { IpcToClient, MessageToClient, Modules } from './gen/tooling_messages.ts';

/// Connect GUI buttons to callback functions.
document.addEventListener('DOMContentLoaded', () => {
  let foo = [
    ['load_module', load_module],
    ['list_modules', list_modules],
    ['list_systems', list_systems],
    ['engine_pause', engine_pause],
    ['tooling_open', tooling_open],
    ['tooling_close', tooling_close],
  ];
  foo.forEach((entry) => {
    const button = document.getElementById(entry[0]);
    if (button) {
      button.onclick = entry[1];
    } else {
      console.error('Button with ID %s not found.', entry[0]);
    }
  });
});

let recent_send_time = performance.now();
let tooling_ipc: WebSocket | null = null;
let prefix = "Tooling IPC:";
let paused_the_engine = false;

tooling_open();

function getRootAsMessageToClient(bb:flatbuffers.ByteBuffer, obj?:MessageToClient):MessageToClient {
  return (obj || new MessageToClient()).__init(bb.readInt32(bb.position()) + bb.position(), bb);
}

function tooling_open() {
  console.log(prefix, 'opening connection');
  recent_send_time = performance.now();
  tooling_ipc = new WebSocket('ws://127.0.0.1:9002');
  tooling_ipc.binaryType = 'arraybuffer'

  document.getElementById("tooling_callout").textContent = "Tooling opening";
  document.getElementById("tooling_close").hidden = true;

  tooling_ipc.onopen = () => {
    console.log(prefix, 'WebSocket connection opened');

    document.getElementById("tooling_callout").textContent = "Connection is open";
    document.getElementById("tooling_close").hidden = false;
  };

  tooling_ipc.onmessage = (event) => {
    const elapsed = performance.now() - recent_send_time;
    const performance_msg = `Round trip time: ${elapsed} ms`;
    console.log(prefix, performance_msg);
    document.getElementById("tooling_elapsed").textContent = performance_msg;
    console.log(prefix, 'Message from server:', event.data);

    let byte_array = new Uint8Array(event.data);
    let buf = new flatbuffers.ByteBuffer(byte_array);
    let msg = getRootAsMessageToClient(buf);;
    let type = msg.messageType();
    if (type == IpcToClient.ModuleLoaded) {
      document.getElementById("modules").innerHTML = "module loaded";
    } else if (type == IpcToClient.ModuleUnloaded) {
      document.getElementById("modules").innerHTML = "module unloaded";
    } else if (type == IpcToClient.ModuleReloaded) {
      document.getElementById("modules").innerHTML = "module reloaded";
    } else if (type == IpcToClient.Modules) {
      let modules = msg.message(new Modules());
      let module_names = [];
      for (let i = 0; i < modules.listLength(); i++) {
        let name = modules.list(i).name();
        module_names.push(name);
      }
      let display = module_names.join("<br>");
      document.getElementById("modules").innerHTML = display;
    } else if (type == IpcToClient.Systems) {
      let systems = msg.message(new Modules());
      let system_names = [];
      for (let i = 0; i < systems.listLength(); i++) {
        let name = systems.list(i).name();
        system_names.push(name);
      }
      let display = system_names.join("<br>");
      document.getElementById("systems").innerHTML = display;
    } else if (type == IpcToClient.NONE) {
      console.log("Error: msg.messageType() == IpcToClient.NONE");
    } else {
      console.log("Error: Missing message type handler.");
    }
  };

  tooling_ipc.onclose = () => {
    console.log(prefix, 'WebSocket connection closed');
    document.getElementById("tooling_callout").textContent = "Connection is closed";
  };

  tooling_ipc.onerror = (error) => {
    console.error(prefix, 'WebSocket error:', error);
  };
}

function tooling_close() {
  console.log(prefix, 'connection closing');
  socket.close();
  document.getElementById("tooling_callout").textContent = "Tooling closing";
  document.getElementById("tooling_show_close").hidden = true;
}

function load_module() {
  const module_path = document.getElementById("module_path").value;
  console.log(prefix, 'requesting to load a module at ', module_path);
  if (tooling_ipc === null) {
    console.log("tooling_ipc == null");
    return;
  }

  let builder = new flatbuffers.Builder(1024);
  let path_offset = builder.createString(module_path);
  let load_offset = LoadModule.createLoadModule(builder, path_offset);
  let msg = MessageToHost.createMessageToHost(builder, IpcToHost.LoadModule, load_offset)
  builder.finish(msg);

  recent_send_time = performance.now();
  tooling_ipc.send(builder.asUint8Array());
  document.getElementById("tooling_callout").textContent = "Requesting List of Systems";
}

function list_modules() {
  console.log(prefix, 'requesting list of modules');
  if (tooling_ipc === null) {
    console.log("tooling_ipc == null");
    return;
  }

  let builder = new flatbuffers.Builder(1024);
  let offset = ListModules.createListModules(builder);
  let msg = MessageToHost.createMessageToHost(builder, IpcToHost.ListModules, offset)
  builder.finish(msg);

  recent_send_time = performance.now();
  tooling_ipc.send(builder.asUint8Array());
  document.getElementById("tooling_callout").textContent = "Requesting List of modules";
}

function list_systems() {
  console.log(prefix, 'requesting list of systems');
  if (tooling_ipc === null) {
    console.log("tooling_ipc == null");
    return;
  }

  let builder = new flatbuffers.Builder(1024);
  let offset = ListSystems.createListSystems(builder);
  let msg = MessageToHost.createMessageToHost(builder, IpcToHost.ListSystems, offset)
  builder.finish(msg);

  recent_send_time = performance.now();
  tooling_ipc.send(builder.asUint8Array());
  document.getElementById("tooling_callout").textContent = "Requesting List of Systems";
}

function engine_pause() {
  console.log(prefix, 'requesting engine pause');
  if (tooling_ipc === null) {
    console.log("tooling_ipc == null");
    return;
  }

  paused_the_engine = !paused_the_engine;

  let builder = new flatbuffers.Builder(1024);
  let offset = EnginePause.createEnginePause(builder, paused_the_engine);
  let msg = MessageToHost.createMessageToHost(builder, IpcToHost.EnginePause, offset)
  builder.finish(msg);

  recent_send_time = performance.now();
  tooling_ipc.send(builder.asUint8Array());
  document.getElementById("tooling_callout").textContent = "Requesting engine pause {paused_the_engine}";
}
