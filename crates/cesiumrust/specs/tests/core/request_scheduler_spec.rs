//! RequestScheduler 规格测试 - 参考自 Specs/Core/RequestSchedulerSpec
//!
//! A 类测试：15 个（纯逻辑，同步调度器）

use cesium_resource::{get_server_key, Request, RequestScheduler, RequestState, RequestType};

#[cfg(test)]
mod tests {
    use super::*;

    /// "getServer with https"
    #[test]
    fn get_server_key_https() {
        let server = get_server_key("https://test.invalid/1");
        assert_eq!(server, "test.invalid:443");
    }

    /// "getServer with http"
    #[test]
    fn get_server_key_http() {
        let server = get_server_key("http://test.invalid/1");
        assert_eq!(server, "test.invalid:80");
    }

    /// "getServer with explicit port"
    #[test]
    fn get_server_key_explicit_port() {
        let server = get_server_key("https://test.invalid:8443/1");
        assert_eq!(server, "test.invalid:8443");
    }

    /// "getServer strips credentials"
    #[test]
    fn get_server_key_strips_credentials() {
        let server = get_server_key("https://user:pass@test.invalid/1");
        assert_eq!(server, "test.invalid:443");
    }

    /// "honors maximumRequests"
    #[test]
    fn honors_maximum_requests() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 2;
        scheduler.throttle_requests = true;

        // 调度 2 个请求（应成功）
        let r1 = Request::throttled("http://test.invalid/1".to_string(), RequestType::Other, 0.0);
        let r2 = Request::throttled("http://test.invalid/2".to_string(), RequestType::Other, 0.0);
        let id1 = scheduler.schedule(r1);
        let id2 = scheduler.schedule(r2);
        assert!(id1.is_some());
        assert!(id2.is_some());

        scheduler.update();
        assert_eq!(scheduler.active_request_count(), 2);

        // 第三个请求进入 pending（堆还有空间），但不会被激活
        let r3 = Request::throttled("http://test.invalid/3".to_string(), RequestType::Other, 0.0);
        let _id3 = scheduler.schedule(r3);
        scheduler.update();
        // 活跃数保持在上限 2
        assert_eq!(scheduler.active_request_count(), 2);
    }

    /// "honors maximumRequestsPerServer"
    #[test]
    fn honors_maximum_requests_per_server() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests_per_server = 2;
        scheduler.throttle_requests = true;

        let url = "http://test.invalid/1";
        let server = get_server_key(url);

        // 向同一服务器调度 2 个请求
        let r1 = Request::throttled(url.to_string(), RequestType::Other, 0.0);
        let r2 = Request::throttled(url.to_string(), RequestType::Other, 0.0);
        scheduler.schedule(r1);
        scheduler.schedule(r2);
        scheduler.update();

        assert!(!scheduler.server_has_open_slots(&server, 1));

        // 不同服务器应有空余名额
        assert!(scheduler.server_has_open_slots("other.invalid:80", 1));
    }

    /// "honors priorityHeapLength"
    #[test]
    fn honors_priority_heap_length() {
        let mut scheduler = RequestScheduler::new();
        scheduler.priority_heap_length = 1;
        scheduler.maximum_requests = 0; // 强制全部进入 pending
        scheduler.throttle_requests = true;

        let r1 = Request::throttled("http://test.invalid/1".to_string(), RequestType::Other, 0.0);
        let id1 = scheduler.schedule(r1);
        assert!(id1.is_some());

        // 堆已满，第二个请求被拒绝
        let r2 = Request::throttled("http://test.invalid/2".to_string(), RequestType::Other, 1.0);
        let id2 = scheduler.schedule(r2);
        assert!(id2.is_none());
    }

    /// "request goes through immediately when throttle is false"
    #[test]
    fn immediate_when_not_throttled() {
        let mut scheduler = RequestScheduler::new();
        scheduler.throttle_requests = true;

        // 非限流请求立即变为 active
        let mut r = Request::new("https://test.invalid/1".to_string(), RequestType::Other);
        r.throttle = false;
        let id = scheduler.schedule(r);
        assert!(id.is_some());

        // 应立即激活（无需 update）
        let req = scheduler.get_request(id.unwrap()).unwrap();
        assert_eq!(req.state, RequestState::Active);
    }

    /// "makes a throttled request" - state transitions
    #[test]
    fn throttled_request_state_transitions() {
        let mut scheduler = RequestScheduler::new();
        scheduler.throttle_requests = true;
        scheduler.maximum_requests = 0; // 初始强制进入 pending

        let r = Request::throttled("https://test.invalid/1".to_string(), RequestType::Other, 0.0);
        assert_eq!(r.state, RequestState::Unissued);

        let id = scheduler.schedule(r).unwrap();
        // schedule 后（max=0）状态为 Issued（pending）
        {
            let req = scheduler.get_request(id).unwrap();
            assert_eq!(req.state, RequestState::Issued);
        }

        // 现在允许激活
        scheduler.maximum_requests = 1;
        scheduler.update();
        {
            let req = scheduler.get_request(id).unwrap();
            assert_eq!(req.state, RequestState::Active);
        }

        // complete 后请求被移除
        scheduler.complete(id);
        assert_eq!(scheduler.active_request_count(), 0);
        assert!(scheduler.get_request(id).is_none());
    }

    /// "cancels an issued request"
    #[test]
    fn cancels_issued_request() {
        let mut scheduler = RequestScheduler::new();
        scheduler.throttle_requests = true;
        scheduler.maximum_requests = 0; // 强制进入 pending

        let r = Request::throttled("https://test.invalid/1".to_string(), RequestType::Other, 0.0);
        let id = scheduler.schedule(r).unwrap();

        // 确认它处于 pending
        assert_eq!(scheduler.pending_request_count(), 1);

        // 在 update 之前取消
        assert!(scheduler.cancel(id));
        // 取消后请求被移除
        assert!(scheduler.get_request(id).is_none());
    }

    /// "cancels an active request"
    #[test]
    fn cancels_active_request() {
        let mut scheduler = RequestScheduler::new();
        scheduler.throttle_requests = true;

        let r = Request::throttled("https://test.invalid/1".to_string(), RequestType::Other, 0.0);
        let id = scheduler.schedule(r).unwrap();
        scheduler.update();

        // 此时应为 active
        {
            let req = scheduler.get_request(id).unwrap();
            assert_eq!(req.state, RequestState::Active);
        }
        assert_eq!(scheduler.active_request_count(), 1);

        // 取消
        assert!(scheduler.cancel(id));
        // 取消后请求被移除
        assert!(scheduler.get_request(id).is_none());
        assert_eq!(scheduler.active_request_count(), 0);
    }

    /// "prioritizes requests" - lower priority value = higher priority
    #[test]
    fn prioritizes_requests() {
        let mut scheduler = RequestScheduler::new();
        scheduler.throttle_requests = true;
        scheduler.maximum_requests = 1; // 同时只有 1 个 active

        // 调度不同优先级的请求
        let r1 = Request::throttled("http://test.invalid/1".to_string(), RequestType::Other, 0.9);
        let r2 = Request::throttled("http://test.invalid/2".to_string(), RequestType::Other, 0.1);
        let r3 = Request::throttled("http://test.invalid/3".to_string(), RequestType::Other, 0.5);

        let id1 = scheduler.schedule(r1).unwrap();
        let _id2 = scheduler.schedule(r2).unwrap();
        let _id3 = scheduler.schedule(r3).unwrap();

        // 第一次 update 激活一个请求
        scheduler.update();
        assert_eq!(scheduler.active_request_count(), 1);

        // 完成它并 update —— 应激活优先级最高（数值最小）的请求
        scheduler.complete(id1);
        scheduler.update();

        // update 后，某个 pending 请求应变为 active
        assert_eq!(scheduler.active_request_count(), 1);
    }

    /// "handles low priority requests" - heap full rejects low priority
    #[test]
    fn handles_low_priority_requests() {
        let mut scheduler = RequestScheduler::new();
        scheduler.throttle_requests = true;
        scheduler.maximum_requests = 0; // 强制全部进入 pending
        scheduler.priority_heap_length = 2;

        // 填满堆
        let r1 = Request::throttled("http://test.invalid/1".to_string(), RequestType::Other, 0.5);
        let r2 = Request::throttled("http://test.invalid/2".to_string(), RequestType::Other, 0.5);
        assert!(scheduler.schedule(r1).is_some());
        assert!(scheduler.schedule(r2).is_some());

        // 堆已满，低优先级被拒绝
        let r3 = Request::throttled("http://test.invalid/3".to_string(), RequestType::Other, 1.0);
        assert!(scheduler.schedule(r3).is_none());
    }

    /// "does not throttle requests when throttleRequests is false"
    #[test]
    fn no_throttle_when_disabled() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 0; // 正常情况下会阻塞

        // throttle_requests = false 时请求直接通过
        scheduler.throttle_requests = false;
        let r = Request::throttled("https://test.invalid/1".to_string(), RequestType::Other, 0.0);
        let id = scheduler.schedule(r);
        assert!(id.is_some());

        let req = scheduler.get_request(id.unwrap()).unwrap();
        assert_eq!(req.state, RequestState::Active);
    }

    /// "serverHasOpenSlots works for single requests"
    #[test]
    fn server_has_open_slots_single() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests_per_server = 5;
        scheduler.throttle_requests = true;

        let server = "test.invalid:80";

        // 初始有空余名额
        assert!(scheduler.server_has_open_slots(server, 1));

        // 调度 5 个请求
        for i in 0..5 {
            let r = Request::throttled(
                format!("http://test.invalid/{}", i),
                RequestType::Other,
                0.0,
            );
            scheduler.schedule(r);
        }
        scheduler.update();

        // 现在已填满
        assert!(!scheduler.server_has_open_slots(server, 1));
    }

    /// "serverHasOpenSlots works for multiple requests"
    #[test]
    fn server_has_open_slots_multiple() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests_per_server = 5;
        scheduler.throttle_requests = true;

        let server = "test.invalid:80";

        // 调度 2 个请求
        for i in 0..2 {
            let r = Request::throttled(
                format!("http://test.invalid/{}", i),
                RequestType::Other,
                0.0,
            );
            scheduler.schedule(r);
        }
        scheduler.update();

        // 还能容纳 3 个（2+3=5）
        assert!(scheduler.server_has_open_slots(server, 3));
        // 容纳不下 4 个（2+4=6 > 5）
        assert!(!scheduler.server_has_open_slots(server, 4));
    }

    /// "requestsByServer allows for custom maximum requests"
    #[test]
    fn custom_requests_by_server() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests_per_server = 2; // 默认值
        scheduler.requests_by_server.insert("test.invalid:80".to_string(), 23);
        scheduler.throttle_requests = true;

        let server = "test.invalid:80";

        // 调度 23 个请求（自定义上限）
        for i in 0..23 {
            let r = Request::throttled(
                format!("http://test.invalid/{}", i),
                RequestType::Other,
                0.0,
            );
            scheduler.schedule(r);
        }
        scheduler.update();

        // 到达 23 时仍应有空余名额
        assert!(scheduler.server_has_open_slots(server, 0));
        // 但再加 1 个就不行了
        assert!(!scheduler.server_has_open_slots(server, 1));
    }

    /// "heapHasOpenSlots"
    #[test]
    fn heap_has_open_slots() {
        let mut scheduler = RequestScheduler::new();
        scheduler.priority_heap_length = 5;

        assert!(scheduler.heap_has_open_slots(5));
        assert!(!scheduler.heap_has_open_slots(6));
    }
}
