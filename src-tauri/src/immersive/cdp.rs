//! 极简 Chrome DevTools Protocol 客户端：一条 WebSocket 连接对应一个页面目标。
//!
//! - `call` 发命令并等待对应 id 的响应
//! - 事件（没有 id 的消息）从 `connect` 返回的接收端读出

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Clone)]
pub struct Cdp {
    tx: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

/// CDP 事件：方法名 + 参数
pub struct Event {
    pub method: String,
    pub params: Value,
}

/// 连接页面目标的 webSocketDebuggerUrl；连接断开时事件接收端随之关闭
pub async fn connect(ws_url: &str) -> Result<(Cdp, mpsc::UnboundedReceiver<Event>), String> {
    let (ws, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .map_err(|e| format!("连接调试端口失败：{e}"))?;
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let pending: Pending = Arc::default();

    // 写：串行发送
    tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
    });

    // 读：响应交给等待者，事件转发出去
    let pending_r = pending.clone();
    tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(text) = msg else { continue };
            let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else { continue };
            if let Some(id) = v.get("id").and_then(Value::as_u64) {
                if let Some(waiter) = pending_r.lock().await.remove(&id) {
                    let r = match v.get("error") {
                        Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("未知错误").to_string()),
                        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = waiter.send(r);
                }
            } else if let Some(method) = v.get("method").and_then(Value::as_str) {
                let params = v.get("params").cloned().unwrap_or(Value::Null);
                if ev_tx.send(Event { method: method.to_string(), params }).is_err() {
                    break;
                }
            }
        }
        // 连接断开：让所有等待中的调用失败
        for (_, w) in pending_r.lock().await.drain() {
            let _ = w.send(Err("连接已断开".into()));
        }
    });

    Ok((Cdp { tx, pending, next_id: Arc::new(AtomicU64::new(1)) }, ev_rx))
}

impl Cdp {
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let msg = json!({ "id": id, "method": method, "params": params }).to_string();
        self.tx.send(Message::text(msg)).map_err(|_| "连接已断开".to_string())?;
        match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
            Ok(Ok(r)) => r.map_err(|e| format!("{method}：{e}")),
            Ok(Err(_)) => Err("连接已断开".into()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("{method}：响应超时"))
            }
        }
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}
