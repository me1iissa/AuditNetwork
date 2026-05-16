import { Sidebar } from "./components/Sidebar";
import { GraphCanvas } from "./components/GraphCanvas";
import { Scrubber } from "./components/Scrubber";
import { DetailPanel } from "./components/DetailPanel";

export default function App() {
  return (
    <div className="app">
      <Sidebar />
      <div className="centre">
        <GraphCanvas />
        <Scrubber />
      </div>
      <DetailPanel />
    </div>
  );
}
