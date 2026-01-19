import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  async function captureScreen() {
    const filename = await invoke("capture_screen");
    console.log(filename);
  }
  return (
    <div>
      <button onClick={captureScreen}>Capture Screen</button>
    </div>
  );
}

export default App;
