use xcap::Monitor;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// Function to capture the screen
#[tauri::command]
async fn capture_screen() -> Result<String, String> {
    // Grap all the screens/monitors
    let monitors = Monitor::all().map_err(|e| e.to_string())?;
    // We want to capture the screen of main display in case of multiple monitors
    if let Some(monitor) = monitors.first(){
    // await the screen capture, xcap returns buffer struct
    let buffer = monitor.capture_image().map_err(|e: xcap::XCapError| e.to_string())?;
    // // convert the buffer to image
    // let image = image::RgbaImage::from_raw(
    //     buffer.width() as u32,
    //     buffer.height() as u32,
    //     buffer.buffer().to_vec()
    // ).ok_or("Failed to convert buffer to image")?;

    // save the image to a file
    let timestamp = chrono::Utc::now().to_string();
    let filename = format!("screenshot_{}.png", timestamp);
    // using reference to the filename so that we can use the variable later
    buffer.save(&filename).map_err(|e: image::ImageError| e.to_string())?;
    // return success
    return Ok(filename);
    }
    return Err("No monitors found".to_string());
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, capture_screen])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
