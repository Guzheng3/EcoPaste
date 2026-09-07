//! QQ 号存在性校验：调用 QQ 空间公开资料接口，确认号码真实存在才在实体栏展示。
//!
//! 提取规则（[`crate::clipboard::entities`]）只能保证「长得像 QQ 号」，无法区分
//! 已注销 / 从未注册的号码——纯数字串误报成 QQ 的概率不低。这里用 Qzone 的
//! 公开资料接口做一次轻量核验：号码存在时返回 `portraitCallBack({"123456":[...]})`，
//! 不存在时返回空对象 `portraitCallBack({})`。
//!
//! 接口不可达 / 超时 / 响应异常时返回 `None`，调用方按「保留显示」降级，
//! 避免网络抖动导致 QQ 实体功能整体失效。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 校验结果缓存：QQ 是否注册长期不变，进程内缓存一份，避免列表滚动反复请求。
static VERIFIED: OnceLock<Mutex<HashMap<String, Option<bool>>>> = OnceLock::new();

/// 校验请求整体超时。Qzone 接口正常响应在数百毫秒内，3 秒足够判断网络可达性。
const VERIFY_TIMEOUT: Duration = Duration::from_secs(3);

/// Qzone 资料接口的 User-Agent：无 UA 的请求可能被网关拒绝。
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) EcoPaste/1.1";

/// 校验 QQ 号是否真实存在。
///
/// 返回 `Some(true)` = 存在；`Some(false)` = 接口明确表示不存在；
/// `None` = 校验失败（网络错误 / 超时 / 响应异常），调用方应保留显示。
pub async fn verify_qq_exists(qq: &str) -> Option<bool> {
    let cache = VERIFIED.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().expect("qq cache poisoned").get(qq) {
        return *cached;
    }

    let owned = qq.to_owned();
    // ureq 是阻塞客户端，放进阻塞线程池避免卡住 tokio worker。
    let result = tauri::async_runtime::spawn_blocking(move || verify_via_qzone(&owned))
        .await
        .ok()
        .flatten();

    cache
        .lock()
        .expect("qq cache poisoned")
        .insert(qq.to_owned(), result);
    result
}

/// 请求 Qzone 公开资料接口并判定号码是否存在。失败返回 `None`（降级保留显示）。
fn verify_via_qzone(qq: &str) -> Option<bool> {
    let agent = ureq::AgentBuilder::new().timeout(VERIFY_TIMEOUT).build();
    let url = format!("https://r.qzone.qq.com/fcg-bin/cgi_get_portrait.fcg?uins={qq}");
    let body = agent
        .get(&url)
        .set("User-Agent", USER_AGENT)
        .call()
        .ok()?
        .into_string()
        .ok()?;

    Some(response_indicates_exists(&body, qq))
}

/// 判定 Qzone 响应是否表明号码存在。
///
/// 响应形如 `portraitCallBack({"123456":["...","昵称",...]})`：昵称部分是 GBK 编码，
/// 但键（QQ 号）、引号、冒号都是 ASCII 字节，直接做字节级包含匹配即可，
/// 无需 GBK 解码。响应不含该键（空对象或异常页）视为不存在。
fn response_indicates_exists(body: &str, qq: &str) -> bool {
    body.contains(&format!("\"{qq}\":"))
}

#[cfg(test)]
mod tests {
    use super::response_indicates_exists;

    #[test]
    fn existing_qq_has_key_in_response() {
        assert!(response_indicates_exists(
            "portraitCallBack({\"123456\":[\"http://qlogo.cn/1\",\"19\",\"19\",\"24\",\"24\",\"0\",\"昵称\",\"0\"]})",
            "123456"
        ));
    }

    #[test]
    fn empty_portrait_object_means_missing() {
        assert!(!response_indicates_exists("portraitCallBack({})", "123456"));
    }

    #[test]
    fn substring_qq_does_not_false_positive() {
        // 12345 是 123456 的子串，但完整键 `"12345":` 不存在，不能误判存在。
        assert!(!response_indicates_exists(
            "portraitCallBack({\"123456\":[\"a\"]})",
            "12345"
        ));
    }
}
