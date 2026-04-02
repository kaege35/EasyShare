use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use walkdir::WalkDir;

pub const TRANSFER_PORT: u16 = 53318;

#[derive(Serialize, Deserialize, Debug)]
pub enum TransferProtocol {
    TransferRequest {
        total_size: u64,
        total_files: u32,
        id: String,
    },
    TransferAccepted,
    TransferDeclined,
    FileHeader {
        rel_path: String,
        file_size: u64,
    },
    AllDone,
}

lazy_static::lazy_static! {
    pub static ref PENDING_TRANSFERS: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>> = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
}

pub async fn start_transfer_server(app: AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], TRANSFER_PORT))).await?;
    let app_clone = app.clone();
    tokio::spawn(async move {
        loop {
            if let Ok((socket, _)) = listener.accept().await {
                let app_c = app_clone.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(socket, app_c).await {
                        println!("TCP Hata: {:?}", e);
                    }
                });
            }
        }
    });
    Ok(())
}

async fn handle_connection(mut socket: TcpStream, app: AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let save_dir = dirs::download_dir().unwrap_or_else(|| std::env::current_dir().unwrap());

    loop {
        let mut len_buf = [0u8; 4];
        let n = socket.read(&mut len_buf).await?;
        if n == 0 { break; }
        if n < 4 { socket.read_exact(&mut len_buf[n..]).await?; }
        
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        socket.read_exact(&mut payload).await?;

        let req: TransferProtocol = serde_json::from_slice(&payload)?;
        
        match req {
            TransferProtocol::TransferRequest { total_size, total_files, id } => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                PENDING_TRANSFERS.lock().await.insert(id.clone(), tx);
                
                let _ = app.emit("transfer-request", serde_json::json!({
                    "id": id,
                    "total_size": total_size,
                    "total_files": total_files
                }));

                let accepted = rx.await.unwrap_or(false);
                let resp = if accepted { TransferProtocol::TransferAccepted } else { TransferProtocol::TransferDeclined };
                let req_json = serde_json::to_vec(&resp)?;
                socket.write_all(&(req_json.len() as u32).to_be_bytes()).await?;
                socket.write_all(&req_json).await?;
                
                if !accepted { return Ok(()); }
            },
            TransferProtocol::TransferAccepted => {},
            TransferProtocol::TransferDeclined => {},
            TransferProtocol::FileHeader { rel_path, file_size } => {
                let mut save_path = save_dir.clone();
                for component in rel_path.split('/') {
                    save_path.push(component);
                }

                if let Some(p) = save_path.parent() {
                    tokio::fs::create_dir_all(p).await?;
                }

                let id = format!("in-{}", rel_path);
                let _ = app.emit("transfer-progress", serde_json::json!({
                    "id": id.clone(),
                    "pct": 0,
                    "text": rel_path.clone(),
                    "is_done": false
                }));

                let mut file = tokio::fs::File::create(&save_path).await?;
                let mut buffer = [0u8; 1024 * 1024]; // 1MB Chunk (Performans için artırıldı)
                let mut remaining = file_size;
                let mut downloaded = 0u64;
                let mut last_pct = 0;
                
                while remaining > 0 {
                    let to_read = std::cmp::min(remaining, buffer.len() as u64) as usize;
                    socket.read_exact(&mut buffer[..to_read]).await?;
                    file.write_all(&buffer[..to_read]).await?;
                    remaining -= to_read as u64;
                    downloaded += to_read as u64;

                    let pct = if file_size == 0 { 100 } else { ((downloaded as f64 / file_size as f64) * 100.0) as u32 };
                    if pct > last_pct || pct == 100 {
                        last_pct = pct;
                        let _ = app.emit("transfer-progress", serde_json::json!({
                            "id": id.clone(),
                            "pct": pct,
                            "text": rel_path.clone(),
                            "is_done": pct == 100
                        }));
                    }
                }

                // Native bildirim gönder (macOS + Windows)
                let _ = app.notification()
                    .builder()
                    .title("EasyShare")
                    .body(format!("İndirme tamamlandı: {}", rel_path))
                    .show();
            },
            TransferProtocol::AllDone => {
                break;
            }
        }
    }
    Ok(())
}

pub async fn send_items(peer_ip: &str, paths: Vec<PathBuf>, app: AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    println!("{} adresine bağlanılıyor...", peer_ip);
    let mut socket = TcpStream::connect(format!("{}:{}", peer_ip, TRANSFER_PORT)).await?;
    println!("Bağlantı başarılı. Dosya listesi hazırlanıyor...");

    let mut all_files = Vec::new();
    let mut total_size = 0u64;
    for p in paths {
        if p.is_file() {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            if let Ok(m) = tokio::fs::metadata(&p).await { total_size += m.len(); }
            all_files.push((name, p));
        } else if p.is_dir() {
            let base_name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            for entry in WalkDir::new(&p).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    if let Ok(rel) = entry.path().strip_prefix(&p) {
                        let mut full_path = PathBuf::from(&base_name);
                        full_path.push(rel);
                        let rel_str = full_path.to_string_lossy().to_string().replace("\\", "/");
                        if let Ok(m) = entry.metadata() { total_size += m.len(); }
                        all_files.push((rel_str, entry.path().to_path_buf()));
                    }
                }
            }
        }
    }

    let req = TransferProtocol::TransferRequest {
        total_size,
        total_files: all_files.len() as u32,
        id: uuid::Uuid::new_v4().to_string(),
    };
    let req_json = serde_json::to_vec(&req)?;
    socket.write_all(&(req_json.len() as u32).to_be_bytes()).await?;
    socket.write_all(&req_json).await?;
    
    let mut len_buf = [0u8; 4];
    socket.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    socket.read_exact(&mut payload).await?;
    let resp: TransferProtocol = serde_json::from_slice(&payload)?;
    
    match resp {
        TransferProtocol::TransferAccepted => {},
        TransferProtocol::TransferDeclined => return Err("ERİŞİM_REDDEDİLDİ".into()),
        _ => return Err("Bilinmeyen yanıt.".into()),
    }

    for (rel_path, abs_path) in all_files {
        let size = tokio::fs::metadata(&abs_path).await?.len();
        
        let id = format!("out-{}", rel_path);
        let _ = app.emit("transfer-out-progress", serde_json::json!({
            "id": id.clone(),
            "pct": 0,
            "text": rel_path.clone(),
            "is_done": false
        }));

        let req = TransferProtocol::FileHeader { rel_path: rel_path.clone(), file_size: size };
        let req_json = serde_json::to_vec(&req)?;
        socket.write_all(&(req_json.len() as u32).to_be_bytes()).await?;
        socket.write_all(&req_json).await?;
        
        let mut file = tokio::fs::File::open(&abs_path).await?;
        let mut buffer = [0u8; 1024 * 1024]; // 1MB Chunk (Hızlı gönderim)
        let mut uploaded = 0u64;
        let mut last_pct = 0;

        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 { break; }
            socket.write_all(&buffer[..n]).await?;
            uploaded += n as u64;

            let pct = if size == 0 { 100 } else { ((uploaded as f64 / size as f64) * 100.0) as u32 };
            if pct > last_pct || pct == 100 {
                last_pct = pct;
                let _ = app.emit("transfer-out-progress", serde_json::json!({
                    "id": id.clone(),
                    "pct": pct,
                    "text": rel_path.clone(),
                    "is_done": pct == 100
                }));
            }
        }
    }

    let end_json = serde_json::to_vec(&TransferProtocol::AllDone)?;
    socket.write_all(&(end_json.len() as u32).to_be_bytes()).await?;
    socket.write_all(&end_json).await?;

    Ok(())
}

