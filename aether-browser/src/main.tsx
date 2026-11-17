import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";
import OverlayHud from "./components/OverlayHud";

const isOverlay = window.location.hash === "#overlay";

createRoot(document.getElementById("root")!).render(<StrictMode>{isOverlay ? <OverlayHud /> : <App />}</StrictMode>);
