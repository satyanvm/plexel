use xcap::Monitor;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// Function to capture the screen
#[tauri::command]
async fn capture_screen() -> Result<String, String> {
    // getting the home directory
    let home_dir = dirs::home_dir().ok_or("Could not find the home directory")?;
    // creating the target directory for storing screenshots: ~/plexel/plexel/screenshots
    let target_dir = home_dir.join("plexel/plexel/screenshots");
    
    // Create the directory if it doesn't exist
    if !target_dir.exists() {
        std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
    }

    // Grab all the screens/monitors
    let monitors = Monitor::all().map_err(|e| e.to_string())?;
    // We want to capture the screen of main display in case of multiple monitors
    if let Some(monitor) = monitors.first(){
        // await the screen capture, xcap returns image struct
        let image = monitor.capture_image().map_err(|e: xcap::XCapError| e.to_string())?;

        // save the image to a file
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("screenshot_{}.png", timestamp);
        let full_path = target_dir.join(filename);
        
        // save using the full path
        image.save(&full_path).map_err(|e: image::ImageError| e.to_string())?;
        
        // return success
        return Ok(full_path.to_string_lossy().to_string());
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
