use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use reqwest;
use rusqlite::{params, Connection, Result as SqlResult};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tokio::task;

const METADATA_URL: &str = "https://raw.githubusercontent.com/xsalazar/emoji-kitchen-backend/main/app/metadata.json";
const DB_FILE: &str = "emoji.db";

// 全局状态，包含更新任务锁
struct AppState {
    update_in_progress: Mutex<bool>,
}

// 查询数据库中合成图片 URL 的函数
fn get_combined_emoji_url_db(emoji1: &str, emoji2: &str) -> SqlResult<Option<String>> {
    let conn = Connection::open(DB_FILE)?;
    // 同时匹配两种排列组合
    let mut stmt = conn.prepare(
        "SELECT gStaticUrl FROM combinations 
         WHERE (leftEmoji = ?1 AND rightEmoji = ?2) 
            OR (leftEmoji = ?2 AND rightEmoji = ?1) 
         LIMIT 1",
    )?;
    let mut rows = stmt.query(params![emoji1, emoji2])?;
    if let Some(row) = rows.next()? {
        let url: String = row.get(0)?;
        Ok(Some(url))
    } else {
        Ok(None)
    }
}

// 初始化数据库：如果文件不存在，则进行更新
async fn init_database() -> Result<(), String> {
    if !Path::new(DB_FILE).exists() {
        update_database().await?;
    }
    Ok(())
}

// 更新数据库的函数（异步版本）
// 这里先用 reqwest::get 下载 metadata，然后通过 spawn_blocking 在独立线程中处理 rusqlite 操作
async fn update_database() -> Result<(), String> {
    // 异步下载 metadata
    let resp = reqwest::get(METADATA_URL)
        .await
        .map_err(|e| format!("网络错误，无法下载喵～：{}", e))?;
    let data = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败喵～：{}", e))?;
    // 保存文件，方便调试
    fs::write("metadata.json", &data).map_err(|e| format!("写入文件失败喵～：{}", e))?;
    let json: Value =
        serde_json::from_str(&data).map_err(|e| format!("解析 JSON 失败喵～：{}", e))?;

    // 将数据库操作放入阻塞线程中执行
    let db_result = task::spawn_blocking(move || -> Result<(), String> {
        let mut conn = Connection::open(DB_FILE)
            .map_err(|e| format!("打开数据库失败喵～：{}", e))?;
        // 创建表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS combinations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                base_emoji TEXT,
                leftEmoji TEXT,
                rightEmoji TEXT,
                gStaticUrl TEXT
            )",
            [],
        )
        .map_err(|e| format!("创建表失败喵～：{}", e))?;
        // 创建索引
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_left_right ON combinations(leftEmoji, rightEmoji)",
            [],
        )
        .map_err(|e| format!("创建索引失败喵～：{}", e))?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_right_left ON combinations(rightEmoji, leftEmoji)",
            [],
        )
        .map_err(|e| format!("创建索引失败喵～：{}", e))?;
        // 清空旧数据
        conn.execute("DELETE FROM combinations", [])
            .map_err(|e| format!("删除旧数据失败喵～：{}", e))?;
        // 解析 JSON 并插入数据
        if let Some(data_obj) = json.get("data").and_then(|v| v.as_object()) {
            let tx = conn
                .transaction()
                .map_err(|e| format!("开启事务失败喵～：{}", e))?;
            for (_key, emoji_data) in data_obj.iter() {
                if let Some(base_emoji) = emoji_data.get("emoji").and_then(|v| v.as_str()) {
                    if let Some(combinations) = emoji_data.get("combinations").and_then(|v| v.as_object()) {
                        for (_combo_key, combo_arr) in combinations.iter() {
                            if let Some(arr) = combo_arr.as_array() {
                                for combo in arr {
                                    let left = combo
                                        .get("leftEmoji")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let right = combo
                                        .get("rightEmoji")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let g_static_url = combo
                                        .get("gStaticUrl")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    tx.execute(
                                        "INSERT INTO combinations (base_emoji, leftEmoji, rightEmoji, gStaticUrl)
                                         VALUES (?1, ?2, ?3, ?4)",
                                        params![base_emoji, left, right, g_static_url],
                                    )
                                    .map_err(|e| format!("插入数据失败喵～：{}", e))?;
                                }
                            }
                        }
                    }
                }
            }
            tx.commit().map_err(|e| format!("提交事务失败喵～：{}", e))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("任务执行失败喵～：{}", e))?;

    db_result
}

// /emoji 接口，查询合成表情图片
async fn emoji_handler(query: web::Query<HashMap<String, String>>) -> impl Responder {
    if let Some(pair) = query.get("pair") {
        println!("查询合成表情图片：{}", pair);
        let parts: Vec<&str> = pair.split('_').collect();
        if parts.len() != 2 {
            return HttpResponse::Ok().body("格式错误喵～".to_string());
        }
        let emoji1 = parts[0].trim();
        let emoji2 = parts[1].trim();
        match get_combined_emoji_url_db(emoji1, emoji2) {
            Ok(Some(url)) => HttpResponse::Ok().body(url),
            Ok(None) => HttpResponse::Ok().body("未找到相关图片喵～".to_string()),
            Err(e) => HttpResponse::Ok().body(format!("查询失败喵～：{}", e)),
        }
    } else {
        HttpResponse::Ok().body("缺少参数喵～".to_string())
    }
}

// /update 接口，更新数据库
async fn update_handler(state: web::Data<AppState>) -> impl Responder {
    {
        let mut lock = state.update_in_progress.lock().unwrap();
        if *lock {
            return HttpResponse::Ok().body("已有更新任务在进行中喵～".to_string());
        }
        *lock = true;
    }

    let result = update_database().await;

    {
        let mut lock = state.update_in_progress.lock().unwrap();
        *lock = false;
    }

    match result {
        Ok(()) => HttpResponse::Ok().body("metadata 更新并同步到数据库成功喵～".to_string()),
        Err(e) => HttpResponse::Ok().body(format!("更新失败喵～：{}", e)),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 初始化数据库（如果不存在则下载并构建数据库）
    if let Err(e) = init_database().await {
        eprintln!("初始化数据库失败喵～：{}", e);
    }
    let state = web::Data::new(AppState {
        update_in_progress: Mutex::new(false),
    });
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .route("/emoji", web::get().to(emoji_handler))
            .route("/update", web::get().to(update_handler))
    })
    .bind("0.0.0.0:21387")?
    .run()
    .await
}