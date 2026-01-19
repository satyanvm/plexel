import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  async function captureScreen() {
    try {
      const filename = await invoke("capture_screen");
      console.log(filename);
    } catch (error) {
      console.error("Error capturing screen:", error);
    }
  }
  return (
    <div>
      <button onClick={captureScreen}>Capture Screen</button>
    </div>
  );
}

export default App;
