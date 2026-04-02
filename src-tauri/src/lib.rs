use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

mod discovery;
mod transfer;

#[tauri::command]
async fn start_discovery(app: AppHandle, name: String) -> Result<(), String> {
    println!("Ağ cihaz keşfi başlatılıyor... Kullanıcı adı: {}", name);
    let id = uuid::Uuid::new_v4().to_string();
    let tcp_port = transfer::TRANSFER_PORT;
    match discovery::start_discovery_loop(app, id, name, tcp_port).await {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Keşif modülü başlatılamadı: {}", e))
    }
}

#[tauri::command]
async fn send_paths_directly(app: AppHandle, peer_ip: String, paths: Vec<String>) -> Result<(), String> {
    let pbs: Vec<std::path::PathBuf> = paths.into_iter().map(std::path::PathBuf::from).collect();
    let app_c = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = transfer::send_items(&peer_ip, pbs, app_c.clone()).await {
            let _ = app_c.emit("transfer-event", format!("Hata: {}", e));
        }
    });
    Ok(())
}

#[tauri::command]
async fn respond_to_transfer(id: String, accept: bool) -> Result<(), String> {
    if let Some(tx) = transfer::PENDING_TRANSFERS.lock().await.remove(&id) {
        let _ = tx.send(accept);
    }
    Ok(())
}

#[tauri::command]
async fn open_file_dialog(app: AppHandle, peer_ip: String) -> Result<(), String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog().file().pick_files(move |file_paths| {
        if let Some(paths) = file_paths {
            let pbs: Vec<std::path::PathBuf> = paths.into_iter().filter_map(|p| p.into_path().ok()).collect();
            let app_c = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = transfer::send_items(&peer_ip, pbs, app_c.clone()).await {
                    let _ = app_c.emit("transfer-event", format!("Hata: {}", e));
                }
            });
        }
    });
    Ok(())
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    if let Some(update) = updater.check().await.map_err(|e| e.to_string())? {
        update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            start_discovery, 
            open_file_dialog,
            send_paths_directly,
            respond_to_transfer,
            install_update
        ])
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let quit_i = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
            let show_i = MenuItem::with_id(app, "show", "EasyShare'i Göster", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("EasyShare")
                .menu(&menu)
                .menu_on_left_click(false)
                .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
                    "quit" => { std::process::exit(0); }
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray: &tauri::tray::TrayIcon, event| match event {
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            // TCP Transfer Sunucusunu başlat
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = transfer::start_transfer_server(handle).await {
                    println!("Transfer server başlatılamadı: {}", e);
                }
            });

            // Açılışta arka planda otomatik güncelleme kontrolü
            let updater_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                use tauri_plugin_updater::UpdaterExt;
                match updater_handle.updater() {
                    Ok(updater) => {
                        match updater.check().await {
                            Ok(Some(update)) => {
                                let version = update.version.clone();
                                let _ = updater_handle.emit("update-available", version);
                            }
                            Ok(None) => { println!("Uygulama güncel."); }
                            Err(e) => { println!("Güncelleme kontrol hatası: {}", e); }
                        }
                    }
                    Err(e) => { println!("Updater başlatılamadı: {}", e); }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let _ = window.hide();
                api.prevent_close();
            }
            _ => {}
        })
        // Duplicate invoke_handler kaldırıldı (üstte tanımlandı)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
