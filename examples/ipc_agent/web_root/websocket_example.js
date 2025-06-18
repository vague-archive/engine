//! Example of connecting to a Fiasco IPC Host.
//!
//! See [`../README.md`] for documentation.

// Create a new WebSocket connection
const socket = new WebSocket('ws://127.0.0.1:9001');
socket.binaryType = 'arraybuffer';

var recent_send_time = performance.now();

socket.onopen = () => {
  console.log('WebSocket connection opened');

  document.getElementById("callout").textContent = "Connection is open";
  document.getElementById("show_close").hidden = false;
};

socket.onmessage = (event) => {
  const elapsed = performance.now() - recent_send_time;
  const msg = `Round trip time: ${elapsed} ms`;
  console.log(msg);
  document.getElementById("elapsed").textContent = msg;
  console.log('Message from server:', event.data);
  const data = new DataView(event.data);
  const kind = data.getUint8(0);
  if (kind == 1) {
    // There is no message of kind 1 in this example. This is here mostly to
    // give an example of switching on some piece of the message (such as the
    // first byte) to determine how to handle the message. There are many ways
    // to define some kind of 'header' to the messages to allow sending
    // different types (kinds) of things on the same socket. Using a byte is
    // just an example. Do what's right for your game.
  } else if (kind == 2) {
    const width = data.getFloat32(1, true);
    const height = data.getFloat32(5, true);
    console.log(`Received screen aspect: ${width}, height ${height}`);
    document.getElementById("aspect").textContent = `Screen aspect width ${width}, height ${height}`;
    document.getElementById("callout").textContent = "Connection is open";
  } else {
    console.log(`Receive an unrecognized message type ${kind}`);
  }
};

socket.onclose = () => {
  console.log('WebSocket connection closed');
  document.getElementById("callout").textContent = "Connection is closed";
};

socket.onerror = (error) => {
  console.error('WebSocket error:', error);
};

function text_example() {
  console.log('WebSocket connection sending text');
  recent_send_time = performance.now();
  socket.send('example text from web page');
  document.getElementById("callout").textContent = "Sent text";
}

function binary_example() {
  console.log('WebSocket connection sending binary');
  recent_send_time = performance.now();
  let message = new Uint8Array([01, 72, 101, 108, 108, 111]);
  socket.send(message);
  document.getElementById("callout").textContent = "Sent binary";
}

function close_connection() {
  console.log('WebSocket connection closing');
  socket.close();
  document.getElementById("callout").textContent = "Closing";
  document.getElementById("show_close").hidden = true;
}

function request_aspect() {
  console.log('WebSocket connection requesting aspect of game screen');
  let message = new Uint8Array([2]);
  recent_send_time = performance.now();
  socket.send(message);
  document.getElementById("callout").textContent = "Requesting Aspect";
}

// Ask the host to stop listening for new connections.
//
// This is a silly thing to do. It's super unlikely that this request would come
// from a remote connection, but it's a way to allow testing of this part of the
// protocol.
function request_ignore() {
  console.log('WebSocket connection requesting ignore new connections');
  let message = new Uint8Array([255]);
  recent_send_time = performance.now();
  socket.send(message);
  document.getElementById("callout").textContent = "Ignoring";
}
