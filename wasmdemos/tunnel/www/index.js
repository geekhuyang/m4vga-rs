import { Demo } from "wasm-tunnel";
import { memory } from "wasm-tunnel/wasm_tunnel_bg";

const demo = Demo.new();
const width = demo.width();
const height = demo.height();

const ptr = demo.framebuffer();
const buffer = new Uint8ClampedArray(memory.buffer, ptr, 4 * width * height);
const image = new ImageData(buffer, width);

const canvas = document.getElementById("demo-canvas");
canvas.height = height;
canvas.width = width;

const ctx = canvas.getContext('2d');

const renderLoop = () => {
  demo.step();

  drawFramebuffer();

  requestAnimationFrame(renderLoop);
};

const drawFramebuffer = () => {
  ctx.putImageData(image, 0, 0);
};

drawFramebuffer();
requestAnimationFrame(renderLoop);