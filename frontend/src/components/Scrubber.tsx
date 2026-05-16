import { useEffect } from "react";
import { useUi } from "../store";
import { closeReplay, openReplay, replay } from "../replay";

const SPEEDS = [0.25, 0.5, 1, 2, 4, 16, 64, 256];

function fmtClock(t: number, base: number | null): string {
  if (base == null) return "—";
  const ms = Math.max(0, t - base);
  const s = Math.floor(ms / 1000);
  const mm = Math.floor(s / 60);
  const ss = s % 60;
  return `${mm.toString().padStart(2, "0")}:${ss.toString().padStart(2, "0")}`;
}

export function Scrubber() {
  const session = useUi((s) => s.selectedSessionId);
  const fromTs = useUi((s) => s.fromTs);
  const toTs = useUi((s) => s.toTs);
  const cursor = useUi((s) => s.cursor);
  const playing = useUi((s) => s.playing);
  const speed = useUi((s) => s.speed);
  const setPlaying = useUi((s) => s.setPlaying);
  const setSpeed = useUi((s) => s.setSpeed);
  const setCursor = useUi((s) => s.setCursor);
  const error = useUi((s) => s.replayError);

  useEffect(() => {
    if (session) openReplay(session);
    return () => closeReplay();
  }, [session]);

  const ready = fromTs != null && toTs != null && cursor != null;
  const onToggle = () => {
    if (playing) {
      replay.pause();
      setPlaying(false);
    } else {
      replay.play();
      setPlaying(true);
    }
  };
  const onSpeed = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const v = parseFloat(e.target.value);
    replay.speed(v);
    setSpeed(v);
  };
  const onScrub = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (!ready) return;
    const v = parseInt(e.target.value, 10);
    replay.seek(v);
    setCursor(v);
  };

  return (
    <div className="scrubber">
      <button onClick={onToggle} disabled={!ready} className="play">
        {playing ? "❚❚" : "▶"}
      </button>
      <select value={speed} onChange={onSpeed} disabled={!ready}>
        {SPEEDS.map((s) => (
          <option key={s} value={s}>
            {s}×
          </option>
        ))}
      </select>
      <span className="time">{fmtClock(cursor ?? 0, fromTs)}</span>
      <input
        type="range"
        min={fromTs ?? 0}
        max={toTs ?? 1}
        step={250}
        value={cursor ?? 0}
        onChange={onScrub}
        disabled={!ready}
      />
      <span className="time">{fmtClock(toTs ?? 0, fromTs)}</span>
      {error && <span className="error-inline">{error}</span>}
    </div>
  );
}
