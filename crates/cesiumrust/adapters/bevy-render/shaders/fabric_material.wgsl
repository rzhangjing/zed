// Fabric procedural material shader.
//
// This is the bevy-render adapter counterpart of the domain `cesium-material`
// crate. The domain layer assembles CesiumJS Fabric GLSL source text; this
// adapter renders the equivalent procedural patterns natively in WGSL so they
// run on Bevy's wgpu pipeline without a runtime GLSL->WGSL transpiler.
//
// Each pattern below is a faithful port of the corresponding CesiumJS material
// shader in `Source/Shaders/Materials/*.glsl`, including the shared built-ins
// `czm_antialias` and `czm_gammaCorrect` (non-HDR path, which is the identity).
//
// The `kind` field of `FabricParams` selects the pattern. Uniform values are
// packed from the domain `Material::uniforms()` map by the Rust adapter.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings

struct FabricParams {
    kind: u32,          // FabricKind discriminant
    horizontal: u32,    // Stripe: 0/1 (GLSL bool horizontal)
    repeat_flag: u32,   // Fade: 0/1 (GLSL bool repeat)
    pixel_ratio: u32,   // Grid: czm_pixelRatio (as u32, typically 1)
    color_a: vec4<f32>, // light/even/color/fadeIn/base/waterColor
    color_b: vec4<f32>, // dark/odd/fadeOut/outlineColor/rimColor/landColor/gapColor
    color_c: vec4<f32>, // Image tint (color)
    repeat_offset: vec4<f32>, // x=repeat.x, y=repeat.y, z=offset(stripe), w=maximumDistance(fade)
    line_params: vec4<f32>,   // x=lineCount.x, y=lineCount.y, z=lineThickness.x, w=lineThickness.y
    line_off_cell: vec4<f32>, // x=lineOffset.x, y=lineOffset.y, z=cellAlpha, w=(spare)
    fade_dir_time: vec4<f32>, // x=fadeDirection.x, y=fadeDirection.y, z=time.x, w=time.y
    // --- Extension fields for materials 7–20 ---
    // x=glowPower, y=taperPower, z=outlineWidth/rimWidth, w=dashLength
    extra_a: vec4<f32>,
    // x=spacing(contour), y=contourWidth, z=strength(normal/bump), w=dashPattern
    extra_b: vec4<f32>,
    // x=minHeight(ramp), y=maxHeight(ramp), z=time(water), w=animationSpeed
    extra_c: vec4<f32>,
    // --- M5-D Water (Water.glsl) uniforms ---
    // x=frequency, y=amplitude, z=specularIntensity, w=fadeFactor
    water_a: vec4<f32>,
}

@group(2) @binding(0) var<uniform> params: FabricParams;
@group(2) @binding(1) var image_texture: texture_2d<f32>;
@group(2) @binding(2) var image_sampler: sampler;
// M5-D: Water multi-texture bindings. normalMap / specularMap are LINEAR data
// (Rgba8Unorm on the Rust side — never Rgba8UnormSrgb, which would double-encode
// the tangent-space normals / water mask). Non-water kinds never sample these.
@group(2) @binding(3) var normal_map: texture_2d<f32>;
@group(2) @binding(4) var normal_sampler: sampler;
@group(2) @binding(5) var specular_map: texture_2d<f32>;
@group(2) @binding(6) var specular_sampler: sampler;

// ---------------------------------------------------------------------------
// Shared built-ins (ported from Shaders/Builtin/Functions/*.glsl)
// ---------------------------------------------------------------------------

// czm_antialias (Shaders/Builtin/Functions/antialias.glsl)
fn czm_antialias(color1: vec4<f32>, color2: vec4<f32>, current_color: vec4<f32>, dist: f32, fuzz_factor: f32) -> vec4<f32> {
    var val1 = clamp(dist / fuzz_factor, 0.0, 1.0);
    let val2 = clamp((dist - 0.5) / fuzz_factor, 0.0, 1.0);
    val1 = val1 * (1.0 - val2);
    var v = val1 * val1 * (3.0 - (2.0 * val1));
    v = pow(v, 0.5); // makes the transition nicer
    let mid_color = (color1 + color2) * 0.5;
    return mix(mid_color, current_color, v);
}

// GLSL mod() (floor modulus); WGSL `%` is remainder, so implement mod faithfully.
fn glsl_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// czm_gammaCorrect (Shaders/Builtin/Functions/gammaCorrect.glsl).
// The domain default is non-HDR, where this is the identity.
fn czm_gamma_correct(color: vec4<f32>) -> vec4<f32> {
    return color;
}

// ---------------------------------------------------------------------------
// M5-D: Water material support (Water.glsl + getWaterNoise.glsl faithful port)
// ---------------------------------------------------------------------------

// METERS_PER_RENDER_UNIT (cesium-geospatial). Water.glsl works in meters; our
// world_position is in render units, so meter-valued constants (the 1e10 fade
// divisor) are divided by this to stay in render-unit space (red-line 米制换算).
const METERS_PER_RENDER_UNIT: f32 = 6378137.0;

// czm_material equivalent: carries the full component set so the Water case can
// populate normal / specular / shininess and be lit by czm_phong. This is an
// *internal* struct — the fragment entry still returns a single vec4 (Bevy's
// forward FragmentOutput is one @location(0) target, not this struct).
struct FabricOutput {
    diffuse: vec3<f32>,
    emission: vec3<f32>,
    alpha: f32,
    normal: vec3<f32>,
    specular: f32,
    shininess: f32,
}

fn fabric_output_default() -> FabricOutput {
    var o: FabricOutput;
    o.diffuse = vec3<f32>(0.0);
    o.emission = vec3<f32>(0.0);
    o.alpha = 1.0;
    o.normal = vec3<f32>(0.0, 0.0, 1.0);
    o.specular = 0.0;
    o.shininess = 1.0;
    return o;
}

// czm_getWaterNoise (Shaders/Builtin/Functions/getWaterNoise.glsl), faithful.
fn czm_get_water_noise(nmap: texture_2d<f32>, nsamp: sampler, uv: vec2<f32>, time: f32, angle_in_radians: f32) -> vec4<f32> {
    let cos_angle = cos(angle_in_radians);
    let sin_angle = sin(angle_in_radians);

    // time dependent sampling directions
    var s0 = vec2<f32>(1.0 / 17.0, 0.0);
    var s1 = vec2<f32>(-1.0 / 29.0, 0.0);
    var s2 = vec2<f32>(1.0 / 101.0, 1.0 / 59.0);
    var s3 = vec2<f32>(-1.0 / 109.0, -1.0 / 57.0);

    // rotate sampling direction by specified angle
    s0 = vec2<f32>((cos_angle * s0.x) - (sin_angle * s0.y), (sin_angle * s0.x) + (cos_angle * s0.y));
    s1 = vec2<f32>((cos_angle * s1.x) - (sin_angle * s1.y), (sin_angle * s1.x) + (cos_angle * s1.y));
    s2 = vec2<f32>((cos_angle * s2.x) - (sin_angle * s2.y), (sin_angle * s2.x) + (cos_angle * s2.y));
    s3 = vec2<f32>((cos_angle * s3.x) - (sin_angle * s3.y), (sin_angle * s3.x) + (cos_angle * s3.y));

    var uv0 = (uv / 103.0) + (time * s0);
    var uv1 = uv / 107.0 + (time * s1) + vec2<f32>(0.23);
    var uv2 = uv / vec2<f32>(897.0, 983.0) + (time * s2) + vec2<f32>(0.51);
    var uv3 = uv / vec2<f32>(991.0, 877.0) + (time * s3) + vec2<f32>(0.71);

    uv0 = fract(uv0);
    uv1 = fract(uv1);
    uv2 = fract(uv2);
    uv3 = fract(uv3);
    let noise = textureSample(nmap, nsamp, uv0)
        + textureSample(nmap, nsamp, uv1)
        + textureSample(nmap, nsamp, uv2)
        + textureSample(nmap, nsamp, uv3);

    // average and scale to between -1 and 1
    return ((noise / 4.0) - 0.5) * 2.0;
}

// czm_getLambertDiffuse / czm_getSpecular / czm_phong (Shaders/Builtin/Functions).
// DEVIATION: CesiumJS evaluates these in eye coordinates (EC) against the
// czm_lightColor / czm_sceneMode uniforms; we evaluate in world coordinates (WC)
// with a fixed white light colour and the 3D-mode horizon branch always taken.
// See docs/deviations.md#dev-019 (M5-D water lighting).
fn czm_get_lambert_diffuse(light_direction: vec3<f32>, normal: vec3<f32>) -> f32 {
    return max(dot(normal, light_direction), 0.0);
}

fn czm_get_specular(light_direction: vec3<f32>, to_eye: vec3<f32>, normal: vec3<f32>, shininess: f32) -> f32 {
    let to_reflected = reflect(-light_direction, normal);
    let specular = max(dot(to_reflected, to_eye), 0.0);
    // pow is undefined when both operands are 0; clamp shininess (phong.glsl).
    return pow(specular, max(shininess, 0.00001));
}

fn czm_phong(to_eye: vec3<f32>, mat: FabricOutput, light_direction: vec3<f32>) -> vec4<f32> {
    let light_color = vec3<f32>(1.0, 1.0, 1.0);
    // Diffuse from a directional light at the eye (top-down) plus the horizon
    // term added in 3D scene mode (phong.glsl L33-37).
    var diffuse = czm_get_lambert_diffuse(vec3<f32>(0.0, 0.0, 1.0), mat.normal);
    diffuse += czm_get_lambert_diffuse(vec3<f32>(0.0, 1.0, 0.0), mat.normal);

    let specular = czm_get_specular(light_direction, to_eye, mat.normal, mat.shininess);

    // Temporary workaround for adding ambient (phong.glsl L42-47).
    let material_diffuse = mat.diffuse * 0.5;
    let ambient = material_diffuse;
    var color = ambient + mat.emission;
    color += material_diffuse * diffuse * light_color;
    color += mat.specular * specular * light_color;

    return vec4<f32>(color, mat.alpha);
}

// ---------------------------------------------------------------------------
// Pattern helpers
// ---------------------------------------------------------------------------

// Fade material getTime() (Shaders/Materials/FadeMaterial.glsl)
fn fade_get_time(t: f32, coord: f32, maximum_distance: f32, repeat: bool) -> f32 {
    let scalar = 1.0 / maximum_distance;
    var q = distance(vec2<f32>(t, coord), vec2<f32>(0.0, 0.0)) * scalar;
    if repeat {
        let r = distance(vec2<f32>(t, coord + 1.0), vec2<f32>(0.0, 0.0)) * scalar;
        let s = distance(vec2<f32>(t, coord - 1.0), vec2<f32>(0.0, 0.0)) * scalar;
        q = min(min(r, s), q);
    }
    return clamp(q, 0.0, 1.0);
}

// ---------------------------------------------------------------------------
// Fragment entry
// ---------------------------------------------------------------------------

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // materialInput.st: use the mesh UVs when present, otherwise fall back to a
    // spherical parameterisation of the world position (unit-sphere showcase).
#ifdef VERTEX_UVS_A
    let st = in.uv;
#else
    let nrm = normalize(in.world_position.xyz);
    let st = vec2<f32>(
        atan2(nrm.z, nrm.x) * 0.15915494309 + 0.5,
        acos(clamp(nrm.y, -1.0, 1.0)) * 0.31830988618);
#endif

    // czm_getDefaultMaterial base values.
    var diffuse = vec3<f32>(0.0, 0.0, 0.0);
    var emission = vec3<f32>(0.0, 0.0, 0.0);
    var alpha = 1.0;

    switch params.kind {
        case 0u: { // Color
            let c = czm_gamma_correct(params.color_a);
            diffuse = c.rgb;
            alpha = c.a;
        }
        case 1u: { // Image
            let uv = fract(params.repeat_offset.xy * st);
            let tex = textureSample(image_texture, image_sampler, uv);
            let c = czm_gamma_correct(vec4<f32>(tex.rgb * params.color_c.rgb, tex.a * params.color_c.a));
            diffuse = c.rgb;
            alpha = c.a;
        }
        case 2u: { // Checkerboard
            let rpt = params.repeat_offset.xy;
            let b = glsl_mod(floor(rpt.x * st.x) + floor(rpt.y * st.y), 2.0); // 0.0 or 1.0
            var scaled_width = fract(rpt.x * st.x);
            scaled_width = abs(scaled_width - floor(scaled_width + 0.5));
            var scaled_height = fract(rpt.y * st.y);
            scaled_height = abs(scaled_height - floor(scaled_height + 0.5));
            let value = min(scaled_width, scaled_height);
            let current_color = mix(params.color_a, params.color_b, b);
            var color = czm_antialias(params.color_a, params.color_b, current_color, value, 0.03);
            color = czm_gamma_correct(color);
            diffuse = color.rgb;
            alpha = color.a;
        }
        case 3u: { // Stripe
            let horizontal_f = select(0.0, 1.0, params.horizontal == 1u);
            let coord = mix(st.x, st.y, horizontal_f);
            let offset = params.repeat_offset.z;
            let rpt = params.repeat_offset.x;
            let value = fract((coord - offset) * (rpt * 0.5));
            let dist = min(value, min(abs(value - 0.5), 1.0 - value));
            let current_color = mix(params.color_a, params.color_b, step(0.5, value));
            var color = czm_antialias(params.color_a, params.color_b, current_color, dist, 0.1);
            color = czm_gamma_correct(color);
            diffuse = color.rgb;
            alpha = color.a;
        }
        case 4u: { // Grid
            let line_count = params.line_params.xy;
            let line_thickness = params.line_params.zw;
            let line_offset = params.line_off_cell.xy;
            let cell_alpha = params.line_off_cell.z;
            let pixel_ratio = f32(params.pixel_ratio);

            var scaled_width = fract(line_count.x * st.x - line_offset.x);
            scaled_width = abs(scaled_width - floor(scaled_width + 0.5));
            var scaled_height = fract(line_count.y * st.y - line_offset.y);
            scaled_height = abs(scaled_height - floor(scaled_height + 0.5));

            // Derivatives branch (GLSL #if __VERSION__==300 path),Listing 4.13
            // from "3D Engine Design for Virtual Globes".
            let fuzz = 1.2;
            let thickness = (line_thickness * pixel_ratio) - vec2<f32>(1.0, 1.0);
            let dx = abs(dpdx(st));
            let dy = abs(dpdy(st));
            let d_f = vec2<f32>(max(dx.x, dy.x), max(dx.y, dy.y)) * line_count;
            var value = min(
                smoothstep(d_f.x * thickness.x, d_f.x * (fuzz + thickness.x), scaled_width),
                smoothstep(d_f.y * thickness.y, d_f.y * (fuzz + thickness.y), scaled_height));

            // Rim suppression (edges taken from RimLightingMaterial.glsl).
            let n = normalize(in.world_normal);
            let view_dir = normalize(view.world_position.xyz - in.world_position.xyz);
            let d_rim = 1.0 - abs(dot(n, view_dir));
            let s_rim = smoothstep(0.8, 1.0, d_rim);
            value = value * (1.0 - s_rim);

            let half_alpha = params.color_a.a * (1.0 - ((1.0 - cell_alpha) * value));
            // WGSL forbids swizzle assignment (`half_color.a = ...`); build the vec4
            // with its final alpha in a single initializer instead (Ryan C1 defense).
            var half_color = vec4<f32>(params.color_a.rgb * 0.5, half_alpha);
            half_color = czm_gamma_correct(half_color);
            diffuse = half_color.rgb;
            emission = half_color.rgb;
            alpha = half_color.a;
        }
        case 5u: { // Dot
            let rpt = params.repeat_offset.xy;
            let b = smoothstep(0.3, 0.32, length(fract(rpt * st) - vec2<f32>(0.5, 0.5))); // 0.0 or 1.0
            var color = mix(params.color_a, params.color_b, b);
            color = czm_gamma_correct(color);
            diffuse = color.rgb;
            alpha = color.a;
        }
        case 6u: { // Fade
            let max_dist = params.repeat_offset.w;
            let rep = params.repeat_flag == 1u;
            let s = fade_get_time(params.fade_dir_time.z, st.x, max_dist, rep) * params.fade_dir_time.x;
            let t = fade_get_time(params.fade_dir_time.w, st.y, max_dist, rep) * params.fade_dir_time.y;
            let u = length(vec2<f32>(s, t));
            var color = mix(params.color_a, params.color_b, u);
            color = czm_gamma_correct(color);
            emission = color.rgb;
            alpha = color.a;
        }
        // ===================================================================
        // New material types (7–20)
        // ===================================================================

        case 7u: { // PolylineArrow (PolylineArrowMaterial.glsl)
            let px_ratio = f32(params.pixel_ratio);
            let fw_st = abs(dpdx(st.x)) + abs(dpdy(st.x));
            let base = 1.0 - fw_st * 10.0 * px_ratio;
            // getPointOnLine for upper arrow edge: (base,1.0) -> center(1.0,0.5)
            let slope_upper = 0.5 / (base - 1.0);
            let pt_on_upper = slope_upper * (st.x - base) + 1.0;
            // getPointOnLine for lower arrow edge: (base,0.0) -> center(1.0,0.5)
            let slope_lower = 0.5 / (base - 1.0);
            let pt_on_lower = slope_lower * (st.x - base);

            let half_width = 0.15;
            var s = step(0.5 - half_width, st.y);
            s *= 1.0 - step(0.5 + half_width, st.y);
            s *= 1.0 - step(base, st.x);

            var t = step(base, st.x);
            t *= 1.0 - step(pt_on_upper, st.y);
            t *= step(pt_on_lower, st.y);

            var dist: f32;
            if st.x < base {
                let d1 = abs(st.y - (0.5 - half_width));
                let d2 = abs(st.y - (0.5 + half_width));
                dist = min(d1, d2);
            } else {
                let d1 = select(1e10, abs(st.x - base), st.y < 0.5 - half_width || st.y > 0.5 + half_width);
                let d2 = abs(st.y - pt_on_upper);
                let d3 = abs(st.y - pt_on_lower);
                dist = min(min(d1, d2), d3);
            }

            let current_color = mix(vec4<f32>(0.0), params.color_a, clamp(s + t, 0.0, 1.0));
            var color = czm_antialias(vec4<f32>(0.0), params.color_a, current_color, dist, 0.03);
            color = czm_gamma_correct(color);
            diffuse = color.rgb;
            alpha = color.a;
        }

        case 8u: { // PolylineDash (PolylineDashMaterial.glsl)
            let dash_length = params.extra_a.w;
            let dash_pattern = params.extra_b.w;
            let px_ratio = f32(params.pixel_ratio);

            // Compute line direction from screen-space derivatives of st
            let dsd = vec2<f32>(dpdx(st).x, dpdy(st).x);
            let angle = atan2(dsd.y, dsd.x);
            let c = cos(-angle);
            let ss = sin(-angle);
            let rot_x = c * st.x + ss * st.y;

            // Dash pattern: 16-bit mask
            let dash_pos = fract(rot_x / (dash_length * px_ratio));
            let mask_idx = floor(dash_pos * 16.0);
            let mask_test = floor(dash_pattern / pow(2.0, mask_idx));
            // glsl_mod, NOT the bare `mod`: WGSL has no `mod` builtin and `mod`
            // is a *reserved word* (naga keywords/wgsl.rs RESERVED). Parsing it as
            // a call silently succeeds, but lowering fails (globals.get("mod")=
            // None → UnknownIdent), which aborts the whole shader and drops all 21
            // Fabric cases from pipeline creation. Ryan C1 fix.
            let on = glsl_mod(mask_test, 2.0) >= 1.0;

            let frag_color = select(params.color_b, params.color_a, on);
            if frag_color.a < 0.005 { discard; }
            let c2 = czm_gamma_correct(frag_color);
            emission = c2.rgb;
            alpha = c2.a;
        }

        case 9u: { // PolylineGlow (PolylineGlowMaterial.glsl)
            let glow_power = params.extra_a.x;
            let taper_power = params.extra_a.y;
            var glow = glow_power / abs(st.y - 0.5) - (glow_power / 0.5);

            if taper_power <= 0.99999 {
                glow *= min(1.0, taper_power / (0.5 - st.x * 0.5) - (taper_power / 0.5));
            }

            // WGSL forbids swizzle assignment; assemble the vec4 from its rgb and a
            // parts in a single initializer (Ryan C1 defense).
            let glow_rgb = max(vec3<f32>(glow - 1.0 + params.color_a.rgb), params.color_a.rgb);
            let glow_a = clamp(glow, 0.0, 1.0) * params.color_a.a;
            var frag_color = czm_gamma_correct(vec4<f32>(glow_rgb, glow_a));
            emission = frag_color.rgb;
            alpha = frag_color.a;
        }

        case 10u: { // PolylineOutline (PolylineOutlineMaterial.glsl)
            let width = params.extra_a.z; // outlineWidth
            // v_width is approximated; in CesiumJS it comes from the vertex shader.
            // We use 1.0 as a default polyline width.
            let v_width = 1.0;
            let half_interior = 0.5 * (v_width - width) / v_width;
            let b = step(0.5 - half_interior, st.y);
            let b2 = b * (1.0 - step(0.5 + half_interior, st.y));

            let d1 = abs(st.y - (0.5 - half_interior));
            let d2 = abs(st.y - (0.5 + half_interior));
            let dist = min(d1, d2);

            let current_color = mix(params.color_b, params.color_a, b2);
            var out_color = czm_antialias(params.color_b, params.color_a, current_color, dist, 0.03);
            out_color = czm_gamma_correct(out_color);
            diffuse = out_color.rgb;
            alpha = out_color.a;
        }

        case 11u: { // ElevationContour (ElevationContourMaterial.glsl)
            // Use world_position.y as proxy for materialInput.height
            let height = in.world_position.y;
            let spacing = params.extra_b.x;
            let contour_w = params.extra_b.y;
            let px_ratio = f32(params.pixel_ratio);

            let distance_to_contour = glsl_mod(height, spacing);

            let dxc = abs(dpdx(height));
            let dyc = abs(dpdy(height));
            let d_f = max(dxc, dyc) * px_ratio * contour_w;
            let contour_a = select(0.0, 1.0, distance_to_contour < d_f);

            let out_color = czm_gamma_correct(vec4<f32>(params.color_a.rgb, contour_a * params.color_a.a));
            diffuse = out_color.rgb;
            alpha = out_color.a;
        }

        case 12u: { // ElevationRamp (ElevationRampMaterial.glsl)
            let height = in.world_position.y;
            let min_h = params.extra_c.x;
            let max_h = params.extra_c.y;
            let scaled = clamp((height - min_h) / (max_h - min_h), 0.0, 1.0);
            let ramp_color = textureSample(image_texture, image_sampler, vec2<f32>(scaled, 0.5));
            let c = czm_gamma_correct(ramp_color);
            diffuse = c.rgb;
            alpha = c.a;
        }

        case 13u: { // AspectRamp (AspectRampMaterial.glsl)
            let nrm = normalize(in.world_normal);
            let aspect = atan2(-nrm.z, nrm.x); // range [-PI, PI]
            let aspect_norm = aspect * 0.15915494309 + 0.5; // map to [0, 1]
            let ramp_color = textureSample(image_texture, image_sampler, vec2<f32>(aspect_norm, 0.5));
            let c = czm_gamma_correct(ramp_color);
            diffuse = c.rgb;
            alpha = c.a;
        }

        case 14u: { // SlopeRamp (SlopeRampMaterial.glsl)
            let nrm = normalize(in.world_normal);
            let up = vec3<f32>(0.0, 1.0, 0.0);
            let slope_angle = acos(clamp(abs(dot(nrm, up)), 0.0, 1.0));
            let slope_norm = slope_angle / 1.57079632679; // divide by PI/2
            let ramp_color = textureSample(image_texture, image_sampler, vec2<f32>(slope_norm, 0.5));
            let c = czm_gamma_correct(ramp_color);
            diffuse = c.rgb;
            alpha = c.a;
        }

        case 15u: { // NormalMap (NormalMapMaterial.glsl)
            let strength = params.extra_b.z;
            let repeat = params.repeat_offset.xy;
            let tex_val = textureSample(image_texture, image_sampler, fract(repeat * st));
            // WGSL forbids swizzle assignment; build the tangent-space normal in one
            // initializer: xy unpacked from [0,1] to [-1,1], z from strength (Ryan C1 defense).
            var nts = vec3<f32>(
                tex_val.xy * 2.0 - 1.0,
                clamp(1.0 - strength, 0.1, 1.0),
            );
            nts = normalize(nts);
            // Approximate eye-space normal: use world_normal as TBN basis
            let wn = normalize(in.world_normal);
            diffuse = wn * 0.5 + 0.5; // visualize normal
            alpha = 1.0;
        }

        case 16u: { // BumpMap (BumpMapMaterial.glsl)
            let strength = params.extra_b.z;
            let repeat = params.repeat_offset.xy;
            let center_uv = fract(repeat * st);
            let center_bump = textureSample(image_texture, image_sampler, center_uv).r;

            // Simple finite difference for bump
            let dx_uv = fract(repeat * (st + vec2<f32>(0.001, 0.0)));
            let right_bump = textureSample(image_texture, image_sampler, dx_uv).r;
            let dy_uv = fract(repeat * (st + vec2<f32>(0.0, 0.001)));
            let top_bump = textureSample(image_texture, image_sampler, dy_uv).r;

            let nts = normalize(vec3<f32>(
                center_bump - right_bump,
                center_bump - top_bump,
                clamp(1.0 - strength, 0.1, 1.0)));
            let wn = normalize(in.world_normal);
            diffuse = nts * 0.5 + 0.5; // visualize perturbed normal
            alpha = 1.0;
        }

        case 17u: { // Water (Water.glsl — faithful port, M5-D)
            let base_water_color = params.color_a;
            let blend_color = params.color_b;
            let frequency = params.water_a.x;
            let amplitude = max(params.water_a.y, 1e-4);
            let specular_intensity = params.water_a.z;
            let fade_factor = params.water_a.w;
            let animation_speed = params.extra_c.w;
            // extra_c.z carries czm_frameNumber (per-frame counter). Water.glsl L18.
            let time = params.extra_c.z * animation_speed;

            // Water.glsl L21: fade from distance-to-eye. The 1e10 divisor is in
            // metres; divide by METERS_PER_RENDER_UNIT to match world_position's
            // render-unit space (red-line 米制换算).
            let position_to_eye = view.world_position.xyz - in.world_position.xyz;
            let fade_divisor = 10000000000.0 / METERS_PER_RENDER_UNIT;
            let fade = max(1.0, (length(position_to_eye) / fade_divisor) * frequency * fade_factor);

            // Water.glsl L23: specular (water/non-water) mask, .r channel.
            let specular_map_value = textureSample(specular_map, specular_sampler, st).r;

            // Water.glsl L26-35: animated 4-octave noise → tangent-space normal.
            let noise = czm_get_water_noise(normal_map, normal_sampler, st * frequency, time, 0.0);
            // WGSL forbids swizzle assignment (`nts.xy /= fade`); fold the fade divide
            // into the initializer's xy (Ryan C1 defense).
            let nts_unfaded = noise.xyz * vec3<f32>(1.0, 1.0, 1.0 / amplitude);
            var normal_tangent_space = vec3<f32>(nts_unfaded.xy / fade, nts_unfaded.z);
            normal_tangent_space = mix(vec3<f32>(0.0, 0.0, 50.0), normal_tangent_space, specular_map_value);
            normal_tangent_space = normalize(normal_tangent_space);

            // Water.glsl L38: alignment ratio of the perturbed normal with +Z.
            let ts_perturbation_ratio = clamp(dot(normal_tangent_space, vec3<f32>(0.0, 0.0, 1.0)), 0.0, 1.0);

            // Water.glsl L52: tangentToEyeMatrix * nts. We build a world-space TBN
            // (DEVIATION: WC not EC — see docs/deviations.md#dev-019) by deriving an
            // orthonormal basis from the world normal. NOTE (Daniel L3): the former
            // `#ifdef VERTEX_TANGENTS` branch read `in.world_tangent`, a field this
            // shader's `VertexOutput` never declares. FabricMaterial sets no
            // VERTEX_TANGENTS shader_def, so naga_oil stripped the branch and it
            // never compiled — but enabling that def would fail *all 21 cases*. The
            // dead, uncompilable branch is removed; only the derived basis remains.
            let n_world = normalize(in.world_normal);
            let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n_world.y) < 0.99);
            let tangent = normalize(cross(helper, n_world));
            let bitangent = cross(n_world, tangent);
            let world_normal = normalize(
                tangent * normal_tangent_space.x
                + bitangent * normal_tangent_space.y
                + n_world * normal_tangent_space.z);

            // Pack the czm_material equivalent (Water.glsl L41-55).
            var mat = fabric_output_default();
            mat.alpha = mix(blend_color.a, base_water_color.a, specular_map_value) * specular_map_value;
            mat.diffuse = mix(blend_color.rgb, base_water_color.rgb, specular_map_value);
            mat.diffuse += 0.1 * ts_perturbation_ratio;
            mat.emission = vec3<f32>(0.0);
            mat.normal = world_normal;
            mat.specular = specular_intensity;
            mat.shininess = 10.0;

            // Light with czm_phong, INLINE for case 17u only so the shared
            // `return diffuse + emission` path for cases 0-16/18-20 is untouched
            // (red-line: no visual change to other cases). Showcase sun in WC.
            let view_dir = normalize(position_to_eye);
            let light_dir = normalize(vec3<f32>(0.4, 0.85, 0.35));
            let lit = czm_phong(view_dir, mat, light_dir);
            diffuse = lit.rgb;
            emission = vec3<f32>(0.0);
            alpha = lit.a;
        }

        case 18u: { // RimLighting (RimLightingMaterial.glsl)
            let rim_width = params.extra_a.z;
            let wn = normalize(in.world_normal);
            let view_dir = normalize(view.world_position.xyz - in.world_position.xyz);
            let d = 1.0 - abs(dot(wn, view_dir));
            let s = smoothstep(1.0 - rim_width, 1.0, d);

            let out_color = czm_gamma_correct(params.color_a);
            let out_rim = czm_gamma_correct(params.color_b);

            diffuse = out_color.rgb;
            emission = out_rim.rgb * s;
            alpha = mix(out_color.a, out_rim.a, s);
        }

        case 19u: { // ElevationBand (ElevationBandMaterial.glsl — simplified stub)
            let height = in.world_position.y;
            let min_h = params.extra_c.x;
            let max_h = params.extra_c.y;
            let scaled = clamp((height - min_h) / (max_h - min_h), 0.0, 1.0);

            // Discrete bands: quantize height into steps
            let num_bands = 5.0;
            let band = floor(scaled * num_bands) / num_bands;
            let ramp_color = textureSample(image_texture, image_sampler, vec2<f32>(band, 0.5));
            let c = czm_gamma_correct(ramp_color);
            diffuse = c.rgb;
            alpha = c.a;
        }

        case 20u: { // WaterMask (WaterMaskMaterial.glsl)
            // Use height threshold as proxy for waterMask
            let height = in.world_position.y;
            let water_level = params.extra_c.x;
            let water_mask = smoothstep(water_level - 0.1, water_level + 0.1, height);
            let out_color = mix(params.color_a, params.color_b, water_mask);
            let c = czm_gamma_correct(out_color);
            diffuse = c.rgb;
            alpha = c.a;
        }

        default: {
            let c = czm_gamma_correct(params.color_a);
            diffuse = c.rgb;
            alpha = c.a;
        }
    }

    return vec4<f32>(diffuse + emission, alpha);
}
