import { useStore } from "./store";
import DropScreen from "./components/DropScreen";
import Editor from "./components/Editor";

export default function App() {
  const stage = useStore((s) => s.stage);
  return (
    <div className="h-full">
      {stage === "editor" ? (
        <Editor />
      ) : (
        <div className="h-full" data-tauri-drag-region>
          <DropScreen />
        </div>
      )}
    </div>
  );
}
