use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

struct AppState {
    metadata: Mutex<Option<Value>>,
}

const METADATA_URL: &str = "https://raw.githubusercontent.com/xsalazar/emoji-kitchen-backend/main/app/metadata.json";
const FILE_PATH: &str = "./metadata.json";

// 加载或者更新 metadata.json 呀～
async fn load_or_update_metadata(state: web::Data<AppState>) -> Result<String, String> {
    let mut metadata_lock = state.metadata.lock().unwrap();
    if !Path::new(FILE_PATH).exists() {
        // 文件不存在，调用 update_metadata 下载文件喵～
        drop(metadata_lock);
        return update_metadata(state).await;
    } else {
        match fs::read_to_string(FILE_PATH) {
            Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                Ok(json) => {
                    *metadata_lock = Some(json);
                    Ok("metadata.json 加载成功喵～".to_string())
                }
                Err(e) => Err(format!("加载 metadata.json 失败喵：{}", e)),
            },
            Err(e) => Err(format!("读取文件失败喵～：{}", e)),
        }
    }
}

// 更新 metadata.json 呀～
async fn update_metadata(state: web::Data<AppState>) -> Result<String, String> {
    let resp = reqwest::get(METADATA_URL)
        .await
        .map_err(|e| format!("网络错误，无法下载喵～：{}", e))?;
    if !resp.status().is_success() {
        return Err("网络错误，无法下载喵～".to_string());
    }
    let data = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败喵～：{}", e))?;
    fs::write(FILE_PATH, &data).map_err(|e| format!("写入文件失败喵～：{}", e))?;
    match serde_json::from_str::<Value>(&data) {
        Ok(json) => {
            let mut metadata_lock = state.metadata.lock().unwrap();
            *metadata_lock = Some(json);
            Ok("metadata.json 更新成功喵～".to_string())
        }
        Err(e) => Err(format!("解析 JSON 失败喵～：{}", e)),
    }
}

// 根据两个 emoji 查找合成图片的 URL 喵～
fn get_combined_emoji_url(json_data: &Value, emoji1: &str, emoji2: &str) -> Option<String> {
    if let Some(data) = json_data.get("data") {
        if let Some(obj) = data.as_object() {
            for (_key, emoji_data) in obj.iter() {
                if emoji_data.get("emoji")?.as_str()? == emoji1 {
                    if let Some(combinations) = emoji_data.get("combinations") {
                        if let Some(combo_obj) = combinations.as_object() {
                            for (_combo_key, combo_arr) in combo_obj.iter() {
                                if let Some(arr) = combo_arr.as_array() {
                                    for combo in arr {
                                        let left = combo.get("leftEmoji")?.as_str()?;
                                        let right = combo.get("rightEmoji")?.as_str()?;
                                        if (left == emoji1 && right == emoji2)
                                            || (left == emoji2 && right == emoji1)
                                        {
                                            return combo.get("gStaticUrl")?.as_str().map(|s| s.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }
    }
    None
}

// /emoji 接口，查询合成表情图片
async fn emoji_handler(
    state: web::Data<AppState>,
    query: web::Query<HashMap<String, String>>,
) -> impl Responder {
    if let Some(pair) = query.get("pair") {
        println!("查询合成表情图片：{}", pair);
        let parts: Vec<&str> = pair.split('_').collect();
        if parts.len() != 2 {
            return HttpResponse::Ok().body("格式错误喵～".to_string());
        }
        let emoji1 = parts[0].trim();
        let emoji2 = parts[1].trim();
        // 加载或者更新 metadata
        if let Err(err) = load_or_update_metadata(state.clone()).await {
            return HttpResponse::Ok().body(err);
        }
        let metadata_lock = state.metadata.lock().unwrap();
        if let Some(ref json_data) = *metadata_lock {
            if let Some(url) = get_combined_emoji_url(json_data, emoji1, emoji2) {
                return HttpResponse::Ok().body(format!("{}", url));
            } else {
                return HttpResponse::Ok().body("未找到相关图片喵～".to_string());
            }
        } else {
            return HttpResponse::Ok().body("metadata 未加载喵～".to_string());
        }
    }
    HttpResponse::Ok().body("缺少参数喵～".to_string())
}

// /update 接口，更新 metadata 文件
async fn update_handler(state: web::Data<AppState>) -> impl Responder {
    match update_metadata(state.clone()).await {
        Ok(msg) => HttpResponse::Ok().body(msg),
        Err(err) => HttpResponse::Ok().body(format!("更新 metadata.json 失败喵：{}", err)),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let state = web::Data::new(AppState {
        metadata: Mutex::new(None),
    });
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .route("/emoji", web::get().to(emoji_handler))
            .route("/update", web::get().to(update_handler))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}