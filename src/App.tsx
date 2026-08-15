import { Component, type ReactNode } from "react";
import { useStore } from "./store";
import DropScreen from "./components/DropScreen";
import Editor from "./components/Editor";

class EditorErrorBoundary extends Component<
  { children: ReactNode; onReset: () => void },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex h-full flex-col items-center justify-center gap-4 px-8">
          <p className="text-lg font-medium text-glow">The editor hit a problem</p>
          <p className="max-w-md text-center text-sm text-dust">
            {this.state.error.message}
          </p>
          <button
            onClick={() => {
              this.props.onReset();
              this.setState({ error: null });
            }}
            className="rounded-md bg-glow px-4 py-1.5 text-sm font-semibold text-well"
          >
            Back to start
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

export default function App() {
  const stage = useStore((s) => s.stage);
  const reset = useStore((s) => s.reset);
  return (
    <div className="h-full min-h-full">
      {stage === "editor" ? (
        <EditorErrorBoundary onReset={reset}>
          <Editor />
        </EditorErrorBoundary>
      ) : (
        <div className="h-full" data-tauri-drag-region>
          <DropScreen />
        </div>
      )}
    </div>
  );
}
