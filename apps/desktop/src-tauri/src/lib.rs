use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use image::RgbaImage;
use leptess::{leptonica, tesseract::TessApi};
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::time::sleep;
use xcap::Monitor;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

const VECTOR_DIMENSIONS: usize = 256;
const DEFAULT_INTERVAL_SECONDS: u64 = 60;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IngestionRecord {
    id: i64,
    captured_at: String,
    image_hash: String,
    text_hash: String,
    extracted_text: String,
    summary: String,
    vector: Vec<f32>,
    word_count: usize,
    line_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitoringStatus {
    running: bool,
    interval_seconds: u64,
    last_run_at: Option<String>,
    last_ingested_at: Option<String>,
    last_error: Option<String>,
    total_items: u64,
    database_path: String,
    last_result: Option<IngestionRecord>,
}

#[derive(Debug)]
struct MonitorStateInner {
    running: bool,
    interval_seconds: u64,
    last_run_at: Option<String>,
    last_ingested_at: Option<String>,
    last_error: Option<String>,
    total_items: u64,
    last_result: Option<IngestionRecord>,
    last_image_hash: Option<String>,
}

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<MonitorStateInner>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MonitorStateInner {
                running: false,
                interval_seconds: DEFAULT_INTERVAL_SECONDS,
                last_run_at: None,
                last_ingested_at: None,
                last_error: None,
                total_items: 0,
                last_result: None,
                last_image_hash: None,
            })),
        }
    }
}

enum IngestionOutcome {
    Stored,
    SkippedUnchanged { image_hash: String },
    SkippedEmpty { image_hash: String },
}

fn data_root() -> Result<PathBuf, String> {
    let home_dir = dirs::home_dir().ok_or("Could not find the home directory")?;
    let root = home_dir.join("plexel/plexel");
    fs::create_dir_all(&root).map_err(|err| err.to_string())?;
    Ok(root)
}

fn screenshots_dir() -> Result<PathBuf, String> {
    let dir = data_root()?.join("screenshots");
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir)
}

fn database_path() -> Result<PathBuf, String> {
    Ok(data_root()?.join("screen_memory.sqlite3"))
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|part| {
            let token = part.trim().to_lowercase();
            if token.is_empty() {
                None
            } else {
                Some(token)
            }
        })
        .collect()
}

fn vectorize_text(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; VECTOR_DIMENSIONS];
    let tokens = tokenize(text);

    if tokens.is_empty() {
        return vector;
    }

    for token in tokens {
        let digest = Sha256::digest(token.as_bytes());
        let index = ((digest[0] as usize) << 8 | digest[1] as usize) % VECTOR_DIMENSIONS;
        vector[index] += 1.0;
    }

    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }

    vector
}

fn summarize_text(text: &str) -> String {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>();

    if lines.is_empty() {
        "No OCR text extracted".to_string()
    } else {
        lines.join(" | ")
    }
}

fn open_database() -> Result<Connection, String> {
    let path = database_path()?;
    let connection = Connection::open(path).map_err(|err| err.to_string())?;
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS screen_memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                captured_at TEXT NOT NULL,
                image_hash TEXT NOT NULL,
                text_hash TEXT NOT NULL,
                extracted_text TEXT NOT NULL,
                summary TEXT NOT NULL,
                vector_json TEXT NOT NULL,
                word_count INTEGER NOT NULL,
                line_count INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_screen_memory_captured_at
            ON screen_memory (captured_at DESC);
            CREATE INDEX IF NOT EXISTS idx_screen_memory_text_hash
            ON screen_memory (text_hash);
            ",
        )
        .map_err(|err| err.to_string())?;
    Ok(connection)
}

fn capture_primary_monitor() -> Result<(RgbaImage, String, PathBuf, String), String> {
    let monitors = Monitor::all().map_err(|err| err.to_string())?;
    let monitor = monitors
        .first()
        .ok_or_else(|| "No monitors found".to_string())?;

    let image = monitor
        .capture_image()
        .map_err(|err: xcap::XCapError| err.to_string())?;
    let image_hash = sha256_hex(image.as_raw());
    let captured_at = now_iso();
    let filename = format!("screenshot_{}.png", Utc::now().format("%Y%m%d_%H%M%S"));
    let path = screenshots_dir()?.join(filename);

    image.save(&path).map_err(|err| err.to_string())?;

    Ok((image, image_hash, path, captured_at))
}

fn run_ocr(image_path: &Path) -> Result<String, String> {
    let mut ocr = TessApi::new(None, "eng")
        .ok_or_else(|| "Failed to initialize the Tesseract OCR engine".to_string())?;
    let mut image = leptonica::pix_read(image_path)
        .ok_or_else(|| "Failed to load screenshot into Leptonica".to_string())?;

    ocr.set_image(&image);
    if ocr.recognize() != 0 {
        image.destroy();
        ocr.destroy();
        return Err("Tesseract failed to recognize text from the screenshot".to_string());
    }

    let text = ocr.get_utf8_text().map_err(|err| err.to_string())?;
    image.destroy();
    ocr.destroy();
    Ok(text)
}

fn insert_record(
    connection: &Connection,
    captured_at: &str,
    image_hash: &str,
    extracted_text: &str,
) -> Result<IngestionRecord, String> {
    let cleaned_text = extracted_text.trim().to_string();
    let text_hash = sha256_hex(cleaned_text.as_bytes());
    let summary = summarize_text(&cleaned_text);
    let vector = vectorize_text(&cleaned_text);
    let vector_json = serde_json::to_string(&vector).map_err(|err| err.to_string())?;
    let word_count = tokenize(&cleaned_text).len();
    let line_count = cleaned_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();

    connection
        .execute(
            "
            INSERT INTO screen_memory (
                captured_at,
                image_hash,
                text_hash,
                extracted_text,
                summary,
                vector_json,
                word_count,
                line_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ",
            params![
                captured_at,
                image_hash,
                text_hash,
                cleaned_text,
                summary,
                vector_json,
                word_count as i64,
                line_count as i64
            ],
        )
        .map_err(|err| err.to_string())?;

    Ok(IngestionRecord {
        id: connection.last_insert_rowid(),
        captured_at: captured_at.to_string(),
        image_hash: image_hash.to_string(),
        text_hash,
        extracted_text: cleaned_text,
        summary,
        vector,
        word_count,
        line_count,
    })
}

fn count_records(connection: &Connection) -> Result<u64, String> {
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM screen_memory", [], |row| row.get(0))
        .map_err(|err| err.to_string())?;
    Ok(count as u64)
}

fn fetch_recent_records(limit: i64) -> Result<Vec<IngestionRecord>, String> {
    let connection = open_database()?;
    let mut statement = connection
        .prepare(
            "
            SELECT
                id,
                captured_at,
                image_hash,
                text_hash,
                extracted_text,
                summary,
                vector_json,
                word_count,
                line_count
            FROM screen_memory
            ORDER BY captured_at DESC
            LIMIT ?1
            ",
        )
        .map_err(|err| err.to_string())?;

    let rows = statement
        .query_map([limit], |row| {
            let vector_json: String = row.get(6)?;
            let vector: Vec<f32> = serde_json::from_str(&vector_json).unwrap_or_default();
            Ok(IngestionRecord {
                id: row.get(0)?,
                captured_at: row.get(1)?,
                image_hash: row.get(2)?,
                text_hash: row.get(3)?,
                extracted_text: row.get(4)?,
                summary: row.get(5)?,
                vector,
                word_count: row.get::<_, i64>(7)? as usize,
                line_count: row.get::<_, i64>(8)? as usize,
            })
        })
        .map_err(|err| err.to_string())?;

    let mut records = Vec::new();
    for row in rows {
        records.push(row.map_err(|err| err.to_string())?);
    }
    Ok(records)
}

fn ingest_once(shared: &Arc<Mutex<MonitorStateInner>>) -> Result<IngestionOutcome, String> {
    let previous_hash = {
        let state = shared
            .lock()
            .map_err(|_| "State lock poisoned".to_string())?;
        state.last_image_hash.clone()
    };

    let (_, image_hash, screenshot_path, captured_at) = capture_primary_monitor()?;
    if previous_hash.as_deref() == Some(image_hash.as_str()) {
        let _ = fs::remove_file(&screenshot_path);
        return Ok(IngestionOutcome::SkippedUnchanged { image_hash });
    }

    let extracted_text = run_ocr(&screenshot_path);
    let _ = fs::remove_file(&screenshot_path);
    let extracted_text = extracted_text?;

    if extracted_text.trim().is_empty() {
        return Ok(IngestionOutcome::SkippedEmpty { image_hash });
    }

    let connection = open_database()?;
    let record = insert_record(&connection, &captured_at, &image_hash, &extracted_text)?;

    {
        let mut state = shared
            .lock()
            .map_err(|_| "State lock poisoned".to_string())?;
        state.last_image_hash = Some(image_hash);
        state.total_items = count_records(&connection)?;
        state.last_ingested_at = Some(captured_at);
        state.last_result = Some(record.clone());
        state.last_error = None;
    }

    Ok(IngestionOutcome::Stored)
}

fn status_from_state(shared: &Arc<Mutex<MonitorStateInner>>) -> Result<MonitoringStatus, String> {
    let database_path = database_path()?.to_string_lossy().to_string();
    let state = shared
        .lock()
        .map_err(|_| "State lock poisoned".to_string())?;

    Ok(MonitoringStatus {
        running: state.running,
        interval_seconds: state.interval_seconds,
        last_run_at: state.last_run_at.clone(),
        last_ingested_at: state.last_ingested_at.clone(),
        last_error: state.last_error.clone(),
        total_items: state.total_items,
        database_path,
        last_result: state.last_result.clone(),
    })
}

async fn monitor_loop(shared: Arc<Mutex<MonitorStateInner>>) {
    loop {
        let interval_seconds = {
            let mut state = match shared.lock() {
                Ok(guard) => guard,
                Err(_) => break,
            };

            if !state.running {
                break;
            }

            state.last_run_at = Some(now_iso());
            state.interval_seconds
        };

        if let Err(error) = ingest_once(&shared) {
            if let Ok(mut state) = shared.lock() {
                state.last_error = Some(error);
            }
        }

        sleep(Duration::from_secs(interval_seconds)).await;
    }
}

#[tauri::command]
async fn process_screenshot_now(
    state: tauri::State<'_, AppState>,
) -> Result<MonitoringStatus, String> {
    {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| "State lock poisoned".to_string())?;
        inner.last_run_at = Some(now_iso());
    }

    match ingest_once(&state.inner)? {
        IngestionOutcome::Stored => status_from_state(&state.inner),
        IngestionOutcome::SkippedUnchanged { image_hash } => {
            {
                let mut inner = state
                    .inner
                    .lock()
                    .map_err(|_| "State lock poisoned".to_string())?;
                inner.last_image_hash = Some(image_hash);
                inner.last_error =
                    Some("Skipped capture because the screen did not change".to_string());
            }
            status_from_state(&state.inner)
        }
        IngestionOutcome::SkippedEmpty { image_hash } => {
            {
                let mut inner = state
                    .inner
                    .lock()
                    .map_err(|_| "State lock poisoned".to_string())?;
                inner.last_image_hash = Some(image_hash);
                inner.last_error = Some("Skipped capture because OCR returned no text".to_string());
            }
            status_from_state(&state.inner)
        }
    }
}

#[tauri::command]
async fn start_monitoring(state: tauri::State<'_, AppState>) -> Result<MonitoringStatus, String> {
    {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| "State lock poisoned".to_string())?;
        if inner.running {
            return status_from_state(&state.inner);
        }
        inner.running = true;
        inner.last_error = None;
    }

    let shared = state.inner.clone();
    tauri::async_runtime::spawn(async move {
        monitor_loop(shared).await;
    });

    status_from_state(&state.inner)
}

#[tauri::command]
fn stop_monitoring(state: tauri::State<'_, AppState>) -> Result<MonitoringStatus, String> {
    {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| "State lock poisoned".to_string())?;
        inner.running = false;
    }

    status_from_state(&state.inner)
}

#[tauri::command]
fn get_monitoring_status(state: tauri::State<'_, AppState>) -> Result<MonitoringStatus, String> {
    status_from_state(&state.inner)
}

#[tauri::command]
fn get_recent_ingestions(limit: Option<u32>) -> Result<Vec<IngestionRecord>, String> {
    fetch_recent_records(limit.unwrap_or(5) as i64)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::default();
    if let Ok(connection) = open_database() {
        if let Ok(total_items) = count_records(&connection) {
            if let Ok(mut inner) = state.inner.lock() {
                inner.total_items = total_items;
            }
        }
    }

    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            process_screenshot_now,
            start_monitoring,
            stop_monitoring,
            get_monitoring_status,
            get_recent_ingestions
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
