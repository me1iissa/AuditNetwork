import { Sidebar } from "./components/Sidebar";
import { GraphCanvas } from "./components/GraphCanvas";

export default function App() {
  return (
    <div className="app">
      <Sidebar />
      <GraphCanvas />
    </div>
  );
}
