import { mount } from "svelte";
import "@xyflow/svelte/dist/style.css";
import "./app.css";
import App from "./App.svelte";

mount(App, { target: document.getElementById("app")! });
console.log("app mounted", document.getElementById("app"));
