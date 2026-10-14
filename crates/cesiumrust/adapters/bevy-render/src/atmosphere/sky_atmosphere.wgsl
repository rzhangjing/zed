// sky_atmosphere.wgsl — single-scattering sky dome fragment shader (M5-C).
//
// ============================================================================
// BLUEPRINT (authoritative source of the algorithm — this is a REWRITE, not a
// line-by-line transpilation; GLSL->WGSL and CesiumJS->Bevy both force it)
// ============================================================================
// B1  cesium-rs/crates/cesium-shaders/shaders/Builtin/Functions/computeScattering.glsl
//     (149 lines / 8329 B), byte-identical to
//     packages/engine/Source/Shaders/Builtin/Functions/computeScattering.glsl (149 lines / 8329 B)
//       L16-24   czm_computeScattering(primaryRay, primaryRayLength, lightDirection,
//                                      atmosphereInnerRadius,
//                                      out rayleighColor, out mieColor, out opacity)
//       L25      const float ATMOSPHERE_THICKNESS = 111e3;
//       L26-27   PRIMARY_STEPS_MAX = 16 / LIGHT_STEPS_MAX = 4
//       L34      atmosphereOuterRadius = atmosphereInnerRadius + ATMOSPHERE_THICKNESS
//       L39-44   czm_raySphereIntersectionInterval + empty-interval early-out
//       L50      x = 1e-7 * stop / length(primaryRayLength)          [see D1 below]
//       L53      w_stop_gt_lprl = 0.5*(1.0 + czm_approximateTanh(x))
//       L56-59   start_0 saved; start = max(start,0); stop = min(stop, primaryRayLength)
//       L64      x_o_a = start_0 - ATMOSPHERE_THICKNESS              [see D2 below]
//       L65      w_inside_atmosphere = 1.0 - 0.5*(1.0 + czm_approximateTanh(x_o_a))
//       L66-67   PRIMARY_STEPS = MAX - int(w_inside*12); LIGHT_STEPS = MAX - int(w_inside*2)
//       L70      rayPositionLength = intersect.start
//       L73      totalRayLength = stop - rayPositionLength
//       L74      rayStepLengthIncrease = w_inside*((1-w_stop_gt_lprl)*totalRayLength
//                                       / (PRIMARY_STEPS*(PRIMARY_STEPS+1)/2))
//       L75      rayStepLength = max(1-w_inside, w_stop_gt_lprl)*totalRayLength
//                               / max(7*w_inside, PRIMARY_STEPS)
//       L77-80   accumulators; heightScale = vec2(rayleighScaleHeight, mieScaleHeight)
//       L83-89   primary march loop (WebGL1 `if (i >= PRIMARY_STEPS) break;` guard)
//       L92      samplePosition = origin + direction*(rayPositionLength + rayStepLength)
//       L95      sampleHeight = length(samplePosition) - atmosphereInnerRadius
//       L98-99   sampleDensity = exp(-sampleHeight/heightScale)*rayStepLength;
//                opticalDepth += sampleDensity
//       L102-105 lightRay + interval; lightStepLength = intersect.stop / LIGHT_STEPS
//       L111-130 light march sub-loop (L120 lightPosition, L123 lightHeight,
//                L126 lightOpticalDepth accumulation)
//       L133     attenuation = exp(-((mieCoeff*(od.y+lightOd.y))
//                                  + (rayleighCoeff*(od.x+lightOd.x))))
//       L136-137 rayleighAccumulation += sampleDensity.x*attenuation;
//                mieAccumulation      += sampleDensity.y*attenuation
//       L140     rayPositionLength += (rayStepLength += rayStepLengthIncrease)
//       L144-145 rayleighColor = czm_atmosphereRayleighCoefficient * rayleighAccumulation;
//                mieColor      = czm_atmosphereMieCoefficient      * mieAccumulation
//       L148     opacity = length(exp(-((mie*od.y) + (rayleigh*od.x))))  [see D3]
// B2  .../Builtin/Functions/approximateTanh.glsl L7-10 (11 lines) — rational tanh
// B3  .../Builtin/Functions/computeGroundAtmosphereScattering.glsl L14-30 (31 lines) —
//     cameraToPositionWC = positionWC - czm_viewerPositionWC -> normalize -> czm_ray;
//     atmosphereInnerRadius = length(positionWC)
// B4  .../Builtin/Functions/raySphereIntersectionInterval.glsl L1-37 (37 lines) —
//     a=dot(d,d), b=2*dot(d,oc), c=dot(oc,oc)-r*r, det=b*b-4ac,
//     t0=(-b-sqrt(det))/(2a), t1=(-b+sqrt(det))/(2a)
// B5  packages/engine/Source/Shaders/Builtin/Functions/computeAtmosphereColor.glsl
//     L15-44 (L60-87 = czm_ray overload, same maths; AtmosphereCommon.glsl L158-187 =
//     master-branch inline copy):
//       L33  rayleighPhase = 3.0/50.2654824574 * (1.0 + cosAngleSq)   [50.2654824574=16pi]
//       L35  miePhase      = 3.0/25.1327412287 * ((1-GSq)*(cosAngleSq+1))
//                            / (pow(1+GSq-2*cosAngle*G, 1.5) * (2+GSq)) [25.1327412287=8pi]
//       L37-41 rayleigh = rayleighPhase*rayleighColor; mie = miePhase*mieColor;
//                color = (rayleigh + mie) * czm_atmosphereLightIntensity
//       L43  return vec4(color, opacity)
// B6  packages/engine/Source/Shaders/SkyAtmosphereFS.glsl L12-59 — the fragment driver
//     that calls computeAtmosphereScattering then computeAtmosphereColor.
//
// CPU REFERENCE (the parity target for mode 1, and the source of the physical
// constants): cesiumrust `domain/atmosphere/src/scattering.rs`
//       L43-73  AtmosphereParameters
//       L81-83  rayleigh_phase  = 3/(16*pi) * (1 + cos^2)      [identical to B5 L33]
//       L90-95  mie_phase       = (1-g^2)(1+cos^2) / (4*pi*(2+g^2)*(1+g^2-2g*cos)^1.5)
//       L102-104 atmospheric_density = exp(-height/scale_height)
//       L118-146 compute_sky_color
//
// ============================================================================
// DELIBERATE DEVIATIONS FROM THE BLUEPRINT ("rewrite, not transpile")
// ============================================================================
// D1  B1 L50 computes `x = 1e-7 * stop / length(primaryRayLength)`. `stop` and
//     `length(primaryRayLength)` are both lengths, so their ratio is a
//     dimensionless number of order 1; multiplying by 1e-7 pins `x ~ 0`, hence
//     `czm_approximateTanh(x) ~ 0` and `w_stop_gt_lprl ~ 0.5` *unconditionally*.
//     The upstream horizon/sky step-split therefore never actually switches.
//     Rewritten here to be driven by the physical quantity B1's own L51-52 doc
//     comment describes (0 = horizon, 1 = sky): the sine of the view-ray
//     elevation above the local horizontal plane. That is a pure dot product of
//     two unit vectors, so it is unit-invariant and deterministic. See
//     `HORIZON_SPLIT_SHARPNESS` and `horizon_split_weight` below.
// D2  B1 L64 computes `x_o_a = start_0 - ATMOSPHERE_THICKNESS`, subtracting one
//     length from another *without normalising*. That makes `w_inside_atmosphere`
//     unit-dependent: in metres `tanh` saturates to +-1, in cesiumrust render
//     units (1 ru = 6378137 m) the same expression yields only +-1.7e-4 and the
//     weight collapses to 0.5 — a different render. B1 L64's own comment calls it
//     "an ad-hoc constant ... only the order of magnitude matters", so it is
//     rewritten as the camera height *normalised by the atmosphere thickness*,
//     which is dimensionless and therefore identical in metres and render units.
// D3  B1 L148 collapses the per-channel transmittance vec3 to a scalar with
//     `length()` (an upstream quirk: it is ~sqrt(3)x the per-channel value and is
//     wavelength-independent). Here the vec3 transmittance is computed per
//     channel and collapsed with the **arithmetic mean** (L1/3) instead, giving
//     `alpha = 1 - mean(transmittance)` for the premultiplied composite
//     `dst = src.rgb + dst * (1 - src.a)`. Be precise about what that buys, or
//     the next maintainer will be misled: `AlphaMode::Premultiplied` carries a
//     *scalar* alpha, so the background (starfield, globe) is extinguished by one
//     number and the per-channel vec3 never reaches the framebuffer. The
//     deviation from B1 is therefore **L1/3 vs L2** (a factor ~sqrt(3) when the
//     three channels are equal), not per-channel vs scalar — B1 is scalar too.
//     What genuinely *is* per-channel is the in-scattering (`radiance`), which
//     the premultiplied blend correctly adds without attenuating it by alpha.
// D4  B1 L25 uses ATMOSPHERE_THICKNESS = 111e3 m; cesiumrust's authoritative
//     physical parameter is `constants::ATMOSPHERE_HEIGHT = 100000.0` m
//     (scattering.rs L16, reaching this shader as `outer_radius - inner_radius`).
//     The domain value wins; the thickness is not hardcoded here at all.
// D5  B1 L83-89 carries a WebGL1-compat `if (i >= PRIMARY_STEPS) break;` because
//     GLSL ES 1.00 forbids non-constant loop bounds. WGSL allows them natively,
//     so the guard is dropped and the bound is the plain loop condition.
// D6  B5 L35 normalises the Mie/Henyey-Greenstein phase with 3/(8*pi);
//     domain scattering.rs L94 normalises the *same* shape with 1/(4*pi) — a
//     constant 1.5x difference, no shape difference. The normalisation is
//     exposed as the `mie_phase_k` uniform and defaults to the domain value
//     (1/(4*pi)) because the acceptance gate for this task is parity with the
//     domain f64 reference. `MIE_PHASE_K_BLUEPRINT` below preserves B5's value
//     for a blueprint-faithful switch. Logged as docs/deviations.md#dev-021.
// D7  B1 L95/L123 compute `sampleHeight = length(samplePosition)
//     - atmosphereInnerRadius` with NO floor. Both the primary ray (when it
//     pierces the Earth: B1 only intersects the *outer* shell at L39) and the
//     light ray (for every sample in Earth's shadow) routinely produce a
//     negative height, so `-sampleHeight/heightScale` reaches +797 for a ray
//     through the Earth's centre and `exp(+797)` is `inf` in f32 (overflow at
//     ~exp(88)). B1 L136-137 then evaluates `sampleDensity.x * attenuation` =
//     `inf * exp(-inf)` = `inf * 0` = **NaN**, which poisons the accumulators
//     and makes the fragment colour non-deterministic — fatal for a
//     pixel-reproducible baseline. Rewritten to floor both heights at 0.0, i.e.
//     cap the density at its sea-level value below the surface (the atmosphere
//     does not extend into the ground). This bounds `sampleDensity <=
//     stepLength` and `opticalDepth <= totalRayLength ~ 2.03`, so the worst
//     extinction is ~700 and `exp(-700)` underflows cleanly to 0.0 — a finite
//     `0.0 * finite` = 0.0, which is also the physically right answer (total
//     shadow). Mirrored op-for-op by `sky_dome::march_single_scattering_f32`.
// D8  B1 L64-65 derive `w_inside_atmosphere` from the *raw* camera height
//     `length(cameraPosition) - atmosphereInnerRadius`. At sea level that
//     difference is zero only when the f32 `length()` happens to round up to
//     exactly `atmosphereInnerRadius`; one ulp below and the weight lands a hair
//     above 0.5, so B1 L66's `int(w_inside * 12.0)` truncates 10.0 to 9 and the
//     step budget *at the surface* becomes a function of rounding. The height is
//     floored at 0.0, which makes the surface case exact (`w_inside == 0.5`,
//     hence 16 - int(6.0) = 10 primary and 4 - int(1.0) = 3 light steps) and
//     reads a camera inside the Earth as being at sea level — the only sensible
//     reading, since the atmosphere does not extend underground. Mirrored by
//     `sky_dome::march_single_scattering_f32`.
// D9  B4 divides by `2*a` with no guard, where `a = dot(direction, direction)`.
//     A zero-length direction makes `a = b = det = 0`, so `t0 = (-0 - 0) / 0` =
//     `0/0` = **NaN**. `max(interval.y, 0.0)` does not filter it: WGSL's `max`
//     returns its first argument unless `e1 < e2`, and `NaN < 0.0` is false, so
//     the NaN flows into `lightStepLength`, then the accumulators, then
//     `radiance` — and under `AlphaMode::Premultiplied` a NaN alpha smears over
//     the entire sky, which the bloom and FXAA neighbourhood taps then spread
//     into pixels that never looked at the degenerate ray at all.
//     Zero directions are reachable, not hypothetical:
//       (i) `SkyAtmosphereParams::from_domain` used to seed `sun_direction =
//           Vec3::ZERO`. It now seeds `Vec3::X`, matching
//           `LightingParams::default()`, which removes the seed itself;
//      (ii) `sky_dome::update_sun_direction` normalises with
//           `normalize_or_zero()`, so any degenerate `LightingParams::sun_direction`
//           (a public resource other systems may write) is pushed through
//           verbatim;
//     (iii) `sky_system` returns early when `AnimationClock` is absent, leaving
//           whatever seed the material was created with in place indefinitely.
//     (i) is fixed at the source; this guard is the defence in depth that makes
//     (ii) and (iii) — and every future caller — finite. A degenerate direction
//     has no interval to report, so the guard returns `EMPTY_INTERVAL`, which
//     both call sites already read as "miss". Mirrored op-for-op by
//     `sky_dome::ray_sphere_interval_f32`.
//
// ============================================================================
// UNITS: metres (domain, f64) -> render units (GPU, f32)
// ============================================================================
// 1 render unit = METERS_PER_RENDER_UNIT = 6378137 m (WGS84 semi-major axis;
// adapters/bevy-render/src/resources.rs L9-13). Lengths are divided by it and
// scattering coefficients are multiplied by it before entering this shader:
//     inner_radius          =  6378137 / 6378137      = 1.0
//     outer_radius          =  6478137 / 6378137      = 1.0156786...
//     rayleigh_scale_height =     8000 / 6378137      = 1.254286e-3
//     mie_scale_height      =     1200 / 6378137      = 1.881429e-4
//     rayleigh_coefficient  = [5.8e-6, 13.5e-6, 33.1e-6] * 6378137
//                           = [36.9932, 86.1048, 211.1165]
//     mie_coefficient       =    21e-6 * 6378137      = 133.9409
// This is *exactly* unit-neutral for every quantity this shader computes:
//     optical_depth = beta * L = (beta_m * MPU) * (L_m / MPU) = beta_m * L_m
//     density       = exp(-h / H)                            = exp(-h_m / H_m)
// Both are ratios, so they are invariant under the rescaling and the f32 GPU
// result differs from the f64 domain result only by f32 rounding (~1e-7
// relative). Proven numerically by `sky_dome.rs` unit tests.
//
// ============================================================================
// NO FMA CONTRACTION
// ============================================================================
// glam fast-math is disabled workspace-wide (Cargo.toml L62-67). Likewise this
// shader must not rely on fused multiply-add contraction: every "product then
// sum" is written as two statements with the intermediate product bound to a
// named `let`, so `a*b + c` keeps its two roundings and stays bit-comparable
// with the Rust f32 mirror `sky_dome::closed_form_sky_color_f32` (which itself
// mirrors domain `compute_sky_color` op-for-op).
//
// The same rule covers the *association* of pure products, which is not free in
// f32 either: multiplication is commutative but not associative, so
// `scattered * solar * exposure` = `(scattered*solar)*exposure` can land 1 ULP
// (~1e-7 relative) away from `scattered * (solar*exposure)`. The mode-0 fragment
// tail therefore binds `let gain = params.solar_intensity * view.exposure;` and
// then multiplies `scattered * gain`, exactly the order
// `sky_dome::shade_sky_f32` uses. Every other product site in this file already
// binds its intermediate (`mie_phase`, `closed_form_radiance`,
// `march_single_scattering`); the gain binding was the one that did not.
//
// ============================================================================
// BLENDING / PREMULTIPLY
// ============================================================================
// The material uses `AlphaMode::Premultiplied`. Bevy's `PREMULTIPLY_ALPHA`
// shader-def is consumed only by bevy_pbr's own `pbr_output` fragment tail; this
// file defines its own `@fragment fn fragment` entry point which fully replaces
// it, so the returned colour is NOT premultiplied a second time. The shader
// therefore returns already-premultiplied `vec4(radiance, alpha)` itself.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings

// ---------------------------------------------------------------------------
// Uniform block — field order/offsets MUST match `SkyAtmosphereParams` in
// adapters/bevy-render/src/atmosphere/sky_dome.rs (encase std140 layout).
// ---------------------------------------------------------------------------
struct SkyAtmosphereParams {
    sun_direction: vec3<f32>,        //   0: unit vector, world space (== ECI, see
                                     //      celestial_system.rs; same source as the
                                     //      DirectionalLight direction)
    inner_radius: f32,               //  12: Earth surface radius, render units (=1.0)
    rayleigh_scale_height: f32,      //  16: 8000 m / MPU
    mie_scale_height: f32,           //  20: 1200 m / MPU
    outer_radius: f32,               //  24: (Earth + 100000 m atmosphere) / MPU
    _pad0: f32,                      //  28
    rayleigh_coefficient: vec3<f32>, //  32: [5.8e-6,13.5e-6,33.1e-6] per metre * MPU
    mie_coefficient: f32,            //  44: 21e-6 per metre * MPU
    mie_anisotropy: f32,             //  48: Henyey-Greenstein g (0.758)
    solar_intensity: f32,            //  52: czm_atmosphereLightIntensity (20.0)
    mie_phase_k: f32,                //  56: Mie phase normalisation (see D6)
    mode: u32,                       //  60: 0 = ray-march, 1 = closed-form parity probe
    primary_steps_max: u32,          //  64: B1 L26 (16)
    light_steps_max: u32,            //  68: B1 L27 (4)
    _pad1: u32,                      //  72
    _pad2: u32,                      //  76
};                                   // size 80, align 16

@group(2) @binding(0) var<uniform> params: SkyAtmosphereParams;

// Production mode: Hillaire/CesiumJS double ray-march single scattering (B1).
const MODE_RAYMARCH: u32 = 0u;
// Verification mode: op-for-op mirror of domain `compute_sky_color` at unit
// density, so a GPU readback can be compared against the f64 CPU reference with
// a tight tolerance. Not a production look. See `sky_dome.rs`.
const MODE_CLOSED_FORM: u32 = 1u;

// B5 L33 / domain scattering.rs L82: 3/(16*pi). The two agree exactly.
const RAYLEIGH_PHASE_K: f32 = 0.05968310365946075;
// B5 L35's 3/(8*pi) normalisation, kept as a named constant (see D6). The
// default uniform value is the domain's 1/(4*pi) = 0.07957747154594767 instead.
const MIE_PHASE_K_BLUEPRINT: f32 = 0.11936620731892150;
// Sharpness of the D1 horizon/sky step-split sigmoid, in units of sin(elevation).
// 8.0 puts the 10%/90% crossover at about +-7.2 degrees of elevation.
const HORIZON_SPLIT_SHARPNESS: f32 = 8.0;
// Sentinel for "the ray misses the atmosphere sphere": stop < start.
const EMPTY_INTERVAL: vec2<f32> = vec2<f32>(1.0, -1.0);
// D9 guard: at or below this, `dot(direction, direction)` counts as zero and the
// interval is reported empty rather than dividing by it. Both production
// directions are normalised before they get here (`ray_direction` in the fragment
// head, `sun_direction` by `update_sun_direction`), so `a` is either 1.0 or
// exactly 0.0 and the threshold only has to separate those two cases.
const DEGENERATE_DIRECTION_EPSILON: f32 = 1.0e-12;

// ---------------------------------------------------------------------------
// B2: czm_approximateTanh — rational approximation, odd, clamped to [-1, 1].
// Mirrored op-for-op by `sky_dome::approximate_tanh_f32`.
// ---------------------------------------------------------------------------
fn approximate_tanh(x: f32) -> f32 {
    let x2 = x * x;
    let numerator = x * (27.0 + x2);
    let denominator = 27.0 + 9.0 * x2;
    return clamp(numerator / denominator, -1.0, 1.0);
}

// ---------------------------------------------------------------------------
// B4: czm_raySphereIntersectionInterval. The sphere is always centred on the
// origin (the Earth centre) in this scene, so B4's `center` argument is elided.
// Returns vec2(start, stop); `stop <= start` means "no intersection" (B4's
// czm_emptyRaySegment), encoded as EMPTY_INTERVAL.
// Mirrored op-for-op by `sky_dome::ray_sphere_interval_f32`.
// ---------------------------------------------------------------------------
fn ray_sphere_interval(origin: vec3<f32>, direction: vec3<f32>, radius: f32) -> vec2<f32> {
    // oc = origin - center, with center = (0,0,0)
    let oc = origin;
    let a = dot(direction, direction);
    // D9: `two_a` below would be exactly 0.0, and `b`/`det` exactly 0.0 too, so
    // `t0 = 0/0 = NaN`. Report a miss instead — see the D9 header note for the
    // full chain by which that NaN would poison the premultiplied sky.
    if (a < DEGENERATE_DIRECTION_EPSILON) {
        return EMPTY_INTERVAL;
    }
    let b = 2.0 * dot(direction, oc);
    let radius_sq = radius * radius;
    let oc_sq = dot(oc, oc);
    let c = oc_sq - radius_sq;
    let b_sq = b * b;
    let four_ac = 4.0 * a * c;
    let det = b_sq - four_ac;
    if (det < 0.0) {
        return EMPTY_INTERVAL;
    }
    let sqrt_det = sqrt(det);
    let two_a = 2.0 * a;
    let t0 = (-b - sqrt_det) / two_a;
    let t1 = (-b + sqrt_det) / two_a;
    return vec2<f32>(t0, t1);
}

// ---------------------------------------------------------------------------
// B5 L33 / domain L82: Rayleigh phase function.
// ---------------------------------------------------------------------------
fn rayleigh_phase(cos_theta: f32) -> f32 {
    let cos_sq = cos_theta * cos_theta;
    let one_plus_cos_sq = 1.0 + cos_sq;
    return RAYLEIGH_PHASE_K * one_plus_cos_sq;
}

// ---------------------------------------------------------------------------
// B5 L35 / domain L90-95: Henyey-Greenstein Mie phase function.
// `k` is the normalisation constant (D6): domain 1/(4*pi) by default,
// blueprint 3/(8*pi) available. Written as two explicit steps so the
// `1 + g^2 - 2*g*cos` polynomial is NOT fused into an FMA.
// Mirrored op-for-op by `sky_dome::mie_phase_f32`.
// ---------------------------------------------------------------------------
fn mie_phase(cos_theta: f32, g: f32, k: f32) -> f32 {
    let g_sq = g * g;
    let cos_sq = cos_theta * cos_theta;
    let one_minus_g_sq = 1.0 - g_sq;
    let one_plus_cos_sq = 1.0 + cos_sq;
    let numerator_a = one_minus_g_sq * one_plus_cos_sq;
    let two_plus_g_sq = 2.0 + g_sq;
    let one_plus_g_sq = 1.0 + g_sq;
    let two_g = 2.0 * g;
    let two_g_cos = two_g * cos_theta;
    let base = one_plus_g_sq - two_g_cos;
    let base_p15 = pow(max(base, 1.0e-20), 1.5);
    let denominator_a = two_plus_g_sq * base_p15;
    let scaled_numerator = k * numerator_a;
    return scaled_numerator / denominator_a;
}

// ---------------------------------------------------------------------------
// B1 L50-53 rewritten per D1: weight in [0, 1], 0.5 at the horizon, ->1 looking
// up at the sky, ->0 looking down at the ground. Drives the primary/light step
// counts and the step-size ramp (B1 L74-75).
// ---------------------------------------------------------------------------
fn horizon_split_weight(sin_elevation: f32) -> f32 {
    let sharpened = sin_elevation * HORIZON_SPLIT_SHARPNESS;
    let t = approximate_tanh(sharpened);
    return 0.5 * (1.0 + t);
}

// D1 helper: sin(elevation) of the ray above the local horizontal plane at the
// camera, i.e. the projection of the unit ray direction onto the local zenith.
fn sin_elevation_at(ray_origin: vec3<f32>, ray_direction: vec3<f32>) -> f32 {
    let origin_length = length(ray_origin);
    // Degenerate origin (exactly at the Earth centre) has no defined zenith.
    if (origin_length < 1.0e-9) {
        return 0.0;
    }
    let up = ray_origin / origin_length;
    return dot(ray_direction, up);
}

fn w_stop_gt_lprl_of(ray_origin: vec3<f32>, ray_direction: vec3<f32>) -> f32 {
    return horizon_split_weight(sin_elevation_at(ray_origin, ray_direction));
}

// ---------------------------------------------------------------------------
// B1 L83-141: the double ray-march. Fills the three out-pointers and returns
// `false` when the primary ray misses the atmosphere entirely (B1 L39-44), in
// which case the caller must emit a fully transparent fragment.
//
// B1's `rayleighAccumulation`/`mieAccumulation` are vec3 because L136-137
// multiplies the scalar `sampleDensity.x/.y` by the vec3 `attenuation`, and
// that attenuation is the only wavelength-dependent factor inside the loop.
// They are kept as vec3 here to preserve B1's structure literally (B1 L144-145
// re-multiplies them by the coefficients to get the final in-scattered
// radiance).
// ---------------------------------------------------------------------------
fn march_single_scattering(
    ray_origin: vec3<f32>,
    ray_direction: vec3<f32>,
    primary_ray_length: f32,
    out_rayleigh_accumulation: ptr<function, vec3<f32>>,
    out_mie_accumulation: ptr<function, vec3<f32>>,
    out_optical_depth: ptr<function, vec2<f32>>,
) -> bool {
    // B1 L39-44: intersect the primary ray with the atmosphere outer shell.
    let interval = ray_sphere_interval(ray_origin, ray_direction, params.outer_radius);
    if (interval.y <= interval.x) {
        return false;
    }

    // B1 L56-59.
    let start_0 = interval.x;
    let start = max(start_0, 0.0);
    let stop = min(interval.y, primary_ray_length);
    if (stop <= start) {
        return false;
    }

    // B1 L64-65 rewritten per D2: dimensionless camera height.
    let thickness = params.outer_radius - params.inner_radius;
    let origin_radius = length(ray_origin);
    let camera_height = max(origin_radius - params.inner_radius, 0.0); // D8
    let camera_height_norm = camera_height / thickness;
    let w_tanh = approximate_tanh(camera_height_norm);
    let w_inside_atmosphere = 1.0 - 0.5 * (1.0 + w_tanh);

    // B1 L66-67. Clamped to >= 1 so the divisions below can never be by zero.
    let primary_steps_f = f32(params.primary_steps_max) - w_inside_atmosphere * 12.0;
    let light_steps_f = f32(params.light_steps_max) - w_inside_atmosphere * 2.0;
    let primary_steps = max(1, i32(primary_steps_f));
    let light_steps = max(1, i32(light_steps_f));

    // B1 L70, L73-75.
    let w_stop_gt_lprl = w_stop_gt_lprl_of(ray_origin, ray_direction);
    let ray_position_length = start;
    let total_ray_length = stop - ray_position_length;
    let tri = f32(primary_steps * (primary_steps + 1));
    let half_tri = tri * 0.5;
    let one_minus_w_inside = 1.0 - w_inside_atmosphere;
    // B1 L74 drives the ramp with `1.0 - w_stop_gt_lprl`, NOT with
    // `1.0 - w_inside_atmosphere`: the ramp is what stretches the steps towards
    // the far end of a near-horizon ray (w_stop_gt_lprl -> 0) and collapses to
    // zero for a straight-up sky ray (w_stop_gt_lprl -> 1). Substituting the
    // camera-altitude weight here would make the ramp depend only on altitude and
    // silently disable the horizon/sky split strategy B1's own L46-49 comment
    // describes.
    let one_minus_w_stop = 1.0 - w_stop_gt_lprl;
    let ramp_numerator = one_minus_w_stop * total_ray_length;
    let ray_step_length_increase = w_inside_atmosphere * (ramp_numerator / half_tri);
    // B1 L75.
    let base_weight = max(one_minus_w_inside, w_stop_gt_lprl);
    let base_numerator = base_weight * total_ray_length;
    let base_denominator = max(7.0 * w_inside_atmosphere, f32(primary_steps));
    let ray_step_length = base_numerator / base_denominator;

    // B1 L77-80.
    let height_scale = vec2<f32>(params.rayleigh_scale_height, params.mie_scale_height);
    var optical_depth = vec2<f32>(0.0, 0.0);
    var rayleigh_accumulation = vec3<f32>(0.0, 0.0, 0.0);
    var mie_accumulation = vec3<f32>(0.0, 0.0, 0.0);
    var cursor = ray_position_length;
    var step_length = ray_step_length;

    // B1 L83-141 (WebGL1 break-guard dropped per D5).
    for (var i = 0; i < primary_steps; i = i + 1) {
        // B1 L92, L95.
        let sample_length = cursor + step_length;
        let sample_position = ray_origin + ray_direction * sample_length;
        let sample_radius = length(sample_position);
        // D7: floored at sea level so no density can overflow to `inf`.
        let sample_height = max(sample_radius - params.inner_radius, 0.0);

        // B1 L98-99.
        let neg_height_over_scale = -sample_height / height_scale;
        let sample_density = exp(neg_height_over_scale) * step_length;
        optical_depth = optical_depth + sample_density;

        // B1 L102-105: the light (sun) ray from the sample point out of the
        // atmosphere.
        let light_direction = params.sun_direction;
        let light_interval = ray_sphere_interval(
            sample_position,
            light_direction,
            params.outer_radius,
        );
        // B1 L105: `lightStepLength = lightRayAtmosphereIntersect.stop / LIGHT_STEPS`.
        // The blueprint divides `.stop` alone, NOT `stop - start`. The light ray
        // always originates *inside* the outer shell, so `start` is negative and
        // subtracting it would march `|start|` (up to a whole Earth diameter)
        // past the atmosphere boundary, stretching the four samples until only
        // their outer half lands in real air and the light optical depth — hence
        // the sunset colour — is badly under-estimated. `max(..., 0.0)` keeps the
        // `(1.0, -1.0)` sentinel of a miss from yielding a negative step.
        let light_stop = max(light_interval.y, 0.0);
        let light_step_length = light_stop / f32(light_steps);

        // B1 L111-130.
        var light_optical_depth = vec2<f32>(0.0, 0.0);
        var light_cursor = 0.0;
        for (var j = 0; j < light_steps; j = j + 1) {
            // B1 L120 samples the *midpoint* of each segment
            // (`lightPositionLength + lightStepLength * 0.5`), which is half a
            // step ahead of plain endpoint sampling and materially more accurate
            // at LIGHT_STEPS_MAX = 4.
            let light_sample_length = light_cursor + light_step_length * 0.5;
            let light_position = sample_position + light_direction * light_sample_length;
            let light_radius = length(light_position);
            // D7: the light ray crosses the Earth for every shadowed sample.
            let light_height = max(light_radius - params.inner_radius, 0.0);
            let light_neg_h_over_scale = -light_height / height_scale;
            light_optical_depth = light_optical_depth
                + exp(light_neg_h_over_scale) * light_step_length;
            // B1 L129: the cursor advances *after* the sample is taken.
            light_cursor = light_cursor + light_step_length;
        }

        // B1 L133: two-way (primary + light) extinction.
        let total_depth = optical_depth + light_optical_depth;
        let mie_depth = params.mie_coefficient * total_depth.y;
        let rayleigh_depth = params.rayleigh_coefficient * total_depth.x;
        let extinction = mie_depth + rayleigh_depth;
        let attenuation = exp(-extinction);

        // B1 L136-137.
        rayleigh_accumulation = rayleigh_accumulation + sample_density.x * attenuation;
        mie_accumulation = mie_accumulation + sample_density.y * attenuation;

        // B1 L140: `rayPositionLength += (rayStepLength += rayStepLengthIncrease)`.
        // GLSL sequences the inner `+=` before the outer one, so the step *grows*
        // first and the cursor then advances by the grown step. The opposite order
        // shifts every sample after the first by one increment — cumulatively
        // ~5 km at the surface, i.e. the order of the 8 km Rayleigh scale height.
        step_length = step_length + ray_step_length_increase;
        cursor = cursor + step_length;
    }

    *out_rayleigh_accumulation = rayleigh_accumulation;
    *out_mie_accumulation = mie_accumulation;
    *out_optical_depth = optical_depth;
    return true;
}

// ---------------------------------------------------------------------------
// Mode 1: op-for-op mirror of domain scattering.rs L118-146 `compute_sky_color`
// at camera_height = 0 (sea level), where `atmospheric_density` is exactly 1.0
// for both species. This is the GPU-vs-CPU parity probe: every operation, in
// every order, matches `sky_dome::closed_form_sky_color_f32`, which in turn
// matches the f64 domain function op-for-op.
//
// Alpha is 1.0 so a premultiplied readback equals the radiance exactly, with no
// background contamination. Radiance is NOT exposure-scaled and NOT clamped
// (domain returns linear values that may exceed 1.0), so the readback needs an
// HDR (16F) framebuffer — see docs/deferred.md.
// ---------------------------------------------------------------------------
fn closed_form_radiance(ray_direction: vec3<f32>) -> vec3<f32> {
    let cos_theta = dot(ray_direction, params.sun_direction);
    let rayleigh_p = rayleigh_phase(cos_theta);
    let mie_p = mie_phase(cos_theta, params.mie_anisotropy, params.mie_phase_k);

    // scattering.rs L131-133 with camera_height = 0.0: exp(-0) = 1.
    let rayleigh_density = 1.0;
    let mie_density = 1.0;

    // scattering.rs L136.
    let path_length = params.outer_radius - params.inner_radius;

    // scattering.rs L139-143, per channel. Each product bound separately so
    // nothing fuses (see the NO FMA CONTRACTION header).
    let beta = params.rayleigh_coefficient;
    let beta_times_density = beta * rayleigh_density;
    let beta_density_phase = beta_times_density * rayleigh_p;
    let rayleigh_term = beta_density_phase * path_length;

    let mie_beta_density = params.mie_coefficient * mie_density;
    let mie_beta_density_phase = mie_beta_density * mie_p;
    let mie_term = mie_beta_density_phase * path_length;

    let summed = rayleigh_term + mie_term;
    return summed * params.solar_intensity;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let world_position = in.world_position.xyz;
    let ray_origin = view.world_position;
    // B3 L14-24: cameraToPositionWC -> normalize -> czm_ray(viewerPositionWC, dir).
    let camera_to_position = world_position - ray_origin;
    let primary_ray_length = length(camera_to_position);
    if (primary_ray_length < 1.0e-9) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let ray_direction = camera_to_position / primary_ray_length;

    // -----------------------------------------------------------------------
    // Mode 1: closed-form CPU-parity probe (see the function's doc comment).
    // -----------------------------------------------------------------------
    if (params.mode == MODE_CLOSED_FORM) {
        return vec4<f32>(closed_form_radiance(ray_direction), 1.0);
    }

    // -----------------------------------------------------------------------
    // Mode 0: production ray-marched single scattering.
    // -----------------------------------------------------------------------
    var rayleigh_accumulation = vec3<f32>(0.0, 0.0, 0.0);
    var mie_accumulation = vec3<f32>(0.0, 0.0, 0.0);
    var optical_depth = vec2<f32>(0.0, 0.0);
    let hit = march_single_scattering(
        ray_origin,
        ray_direction,
        primary_ray_length,
        &rayleigh_accumulation,
        &mie_accumulation,
        &optical_depth,
    );
    // B1 L42-44: no intersection with the atmosphere -> fully transparent.
    if (!hit) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // B1 L144-145.
    let rayleigh_color = params.rayleigh_coefficient * rayleigh_accumulation;
    let mie_color = params.mie_coefficient * mie_accumulation;

    // B5 L33-41.
    let cos_theta = dot(ray_direction, params.sun_direction);
    let rayleigh_p = rayleigh_phase(cos_theta);
    let mie_p = mie_phase(cos_theta, params.mie_anisotropy, params.mie_phase_k);
    let rayleigh_scattered = rayleigh_p * rayleigh_color;
    let mie_scattered = mie_p * mie_color;
    let scattered = rayleigh_scattered + mie_scattered;
    // NO FMA CONTRACTION, and no re-association either: bind the scalar gain
    // first so this is `scattered * (solar * exposure)`, bit-for-bit what
    // `sky_dome::shade_sky_f32` computes. Written as
    // `scattered * solar_intensity * view.exposure` it would associate the other
    // way and differ by up to 1 ULP.
    let gain = params.solar_intensity * view.exposure;
    let radiance = scattered * gain;

    // D3: the in-scattering above stays per-channel; the background attenuation
    // does not, because `AlphaMode::Premultiplied` blends
    // `dst = src.rgb + dst * (1 - src.a)` with a **scalar** `src.a`. That scalar
    // is the arithmetic mean (L1/3) of the per-channel transmittance. B1 L148
    // collapses to a scalar too, but with `length()` (L2) — so the deviation is
    // L1/3 vs L2, not per-channel vs scalar. The per-channel vec3 is still worth
    // computing: it is what the physics says, and it is what the Rust mirror and
    // the unit tests check.
    let total_mie_depth = params.mie_coefficient * optical_depth.y;
    let total_rayleigh_depth = params.rayleigh_coefficient * optical_depth.x;
    let transmittance = exp(-(total_mie_depth + total_rayleigh_depth));
    let one_third = vec3<f32>(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0);
    let mean_transmittance = dot(transmittance, one_third);
    let alpha = clamp(1.0 - mean_transmittance, 0.0, 1.0);

    return vec4<f32>(radiance, alpha);
}
