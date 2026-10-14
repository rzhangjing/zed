//! 相机飞行系统：处理 FlyTo 请求并逐帧推进飞行动画直至完成。

use bevy::prelude::*;
use cesium_interaction::CameraFlight;

use crate::camera::components::{ActiveFlight, CesiumCamera, FlightComplete, FlyToRequest};

/// 处理 FlyToRequest 事件并推进活飞行动画。
///
/// # 参数
/// - `cameras`：待驱动的领域相机查询
/// - `active_flight`：当前活飞行状态资源
/// - `fly_requests`：新 fly-to 请求事件读取器
/// - `flight_complete`：飞行完成事件写入器
/// - `time`：帧间隔时间源
pub fn camera_flight_system(
    mut cameras: Query<&mut CesiumCamera>,
    mut active_flight: ResMut<ActiveFlight>,
    mut fly_requests: EventReader<FlyToRequest>,
    mut flight_complete: EventWriter<FlightComplete>,
    time: Res<Time>,
) {
    let dt = time.delta_secs() as f64;

    // --- 处理新的 fly-to 请求 ---
    // 为每个请求基于当前位置与目标构造一条插值飞行轨迹。
    for request in fly_requests.read() {
        for cesium_cam in cameras.iter() {
            // 至少保留一个极小时长避免除零，目标采用大地经纬度。
            let flight = CameraFlight::fly_to_cartographic(
                &cesium_cam.camera,
                &request.destination,
                &cesium_geospatial::Ellipsoid::WGS84,
                request.duration_secs.max(0.001),
            );
            active_flight.flight = Some(flight);
        }
    }

    // --- 推进活飞行 ---
    // 逐帧将相机沿轨迹推进，一旦轨迹标记完成或返回 false 则结束。
    let mut is_done = false;
    if let Some(flight) = active_flight.flight.as_mut() {
        if flight.complete {
            is_done = true;
        } else {
            for mut cesium_cam in cameras.iter_mut() {
                let still_flying = flight.apply_to_camera(&mut cesium_cam.camera, dt);
                if !still_flying {
                    is_done = true;
                }
            }
        }
    }
    if is_done {
        // 飞行结束：清空活飞行并广播完成事件供其他系统响应。
        active_flight.flight = None;
        flight_complete.send(FlightComplete);
    }
}
