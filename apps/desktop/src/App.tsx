import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type IngestionRecord = {
  id: number;
  capturedAt: string;
  imageHash: string;
  textHash: string;
  extractedText: string;
  summary: string;
  vector: number[];
  wordCount: number;
  lineCount: number;
};

type MonitoringStatus = {
  running: boolean;
  intervalSeconds: number;
  lastRunAt: string | null;
  lastIngestedAt: string | null;
  lastError: string | null;
  totalItems: number;
  databasePath: string;
  lastResult: IngestionRecord | null;
};

function App() {
  const [status, setStatus] = useState<MonitoringStatus | null>(null);
  const [recentItems, setRecentItems] = useState<IngestionRecord[]>([]);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  async function refreshStatus() {
    try {
      const [nextStatus, nextRecentItems] = await Promise.all([
        invoke<MonitoringStatus>("get_monitoring_status"),
        invoke<IngestionRecord[]>("get_recent_ingestions", { limit: 5 }),
      ]);
      setStatus(nextStatus);
      setRecentItems(nextRecentItems);
      setErrorMessage(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setErrorMessage(message);
    }
  }

  useEffect(() => {
    refreshStatus();

    const intervalId = window.setInterval(() => {
      refreshStatus();
    }, 5000);

    return () => {
      window.clearInterval(intervalId);
    };
  }, []);

  async function runAction<T>(actionName: string, action: () => Promise<T>) {
    setBusyAction(actionName);
    try {
      await action();
      await refreshStatus();
      setErrorMessage(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setErrorMessage(message);
    } finally {
      setBusyAction(null);
    }
  }

  return (
    <main className="dashboard">
      <section className="hero">
        <p className="eyebrow">Plexel Screen Memory</p>
        <h1>Capture, OCR, vectorize, and clean up every minute.</h1>
        <p className="intro">
          The desktop worker captures the primary display, extracts text with OCR,
          stores the resulting vector and metadata in a local SQLite-backed vector
          store, then deletes the screenshot file.
        </p>
        <div className="actions">
          <button
            onClick={() =>
              runAction("start", () => invoke("start_monitoring"))
            }
            disabled={busyAction !== null || status?.running}
          >
            Start monitoring
          </button>
          <button
            className="secondary"
            onClick={() => runAction("stop", () => invoke("stop_monitoring"))}
            disabled={busyAction !== null || !status?.running}
          >
            Stop monitoring
          </button>
          <button
            className="secondary"
            onClick={() =>
              runAction("process", () => invoke("process_screenshot_now"))
            }
            disabled={busyAction !== null}
          >
            Process now
          </button>
        </div>
      </section>

      <section className="grid">
        <article className="panel">
          <p className="panel-label">Worker status</p>
          <h2>{status?.running ? "Running" : "Stopped"}</h2>
          <dl className="stats">
            <div>
              <dt>Interval</dt>
              <dd>{status?.intervalSeconds ?? 60}s</dd>
            </div>
            <div>
              <dt>Total records</dt>
              <dd>{status?.totalItems ?? 0}</dd>
            </div>
            <div>
              <dt>Last run</dt>
              <dd>{status?.lastRunAt ?? "Not yet"}</dd>
            </div>
            <div>
              <dt>Last stored</dt>
              <dd>{status?.lastIngestedAt ?? "Not yet"}</dd>
            </div>
          </dl>
        </article>

        <article className="panel">
          <p className="panel-label">Local store</p>
          <h2>SQLite vector memory</h2>
          <p className="path">{status?.databasePath ?? "Loading..."}</p>
          <p className="muted">
            Each row stores OCR text, a lightweight normalized text vector,
            hashes, and basic metadata for later retrieval.
          </p>
        </article>
      </section>

      <section className="panel">
        <p className="panel-label">Latest OCR result</p>
        <h2>{status?.lastResult?.summary ?? "No OCR record yet"}</h2>
        <p className="muted">
          {status?.lastResult
            ? `${status.lastResult.wordCount} words across ${status.lastResult.lineCount} non-empty lines`
            : "Start monitoring or run a single pass to ingest the current screen."}
        </p>
        <pre className="ocr-preview">
          {status?.lastResult?.extractedText ?? "OCR text will appear here."}
        </pre>
      </section>

      <section className="panel">
        <p className="panel-label">Recent records</p>
        <div className="record-list">
          {recentItems.length === 0 ? (
            <p className="muted">No stored records yet.</p>
          ) : (
            recentItems.map((item) => (
              <article key={item.id} className="record-card">
                <p className="record-time">{item.capturedAt}</p>
                <h3>{item.summary}</h3>
                <p className="muted">
                  {item.wordCount} words • {item.lineCount} lines
                </p>
              </article>
            ))
          )}
        </div>
      </section>

      {(errorMessage || status?.lastError) && (
        <section className="panel error-panel">
          <p className="panel-label">Latest note</p>
          <p>{errorMessage ?? status?.lastError}</p>
        </section>
      )}
    </main>
  );
}

export default App;
