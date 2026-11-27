import { mount } from "svelte";
import App from "./App.svelte";
import { bootLook } from "./look";
import "./app.css";

bootLook();
mount(App, { target: document.getElementById("app")! });
