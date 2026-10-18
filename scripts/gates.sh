#!/usr/bin/env bash
# 把 `docs/设计方案.md` §10.5 的 CI 流水线在**本机**可复现地跑一遍（Linux / 麒麟侧）。
#
# 与 `scripts/gates.ps1` 逐阶段对齐；两边都是「一条真命令 + 非零退出码即判红」，
# 日志落在 `target/gates/NN-<阶段>.log`，脚本不造假证据。
#
# 用法：
#   chmod +x scripts/gates.sh          # 首次使用（仓库里的 mode 位可能没带可执行）
#   scripts/gates.sh                    # 离线 + 交叉 + 真机（DM_TEST_DSN 已设时）
#   scripts/gates.sh --skip-real        # 只跑离线 + 交叉（CI 的默认形态）
#   scripts/gates.sh --only fmt,clippy  # 只跑名字匹配的阶段
#   scripts/gates.sh --skip-cross --skip-soak
#   scripts/gates.sh --strict-suite      # 官方套件也要求全绿（关掉已知缺陷白名单）
#
# 关于官方一致性套件：它有一条已登记在册的缺陷（非 BMP 文本，
# `type_primitives::ty_str`，见 docs/toasty-fork.md「真机实测发现」第 16 条），
# 所以该阶段按「失败条数恰为 1 且失败名在白名单里」判 PASS，并把放行的那一条打印出来；
# 要全绿请加 `--strict-suite`。
#
# 关于 `--test-threads=1`：真机阶段不可省 —— 这些用例共用固定表名（DM_Q_REAL_SEED、
# 迁移记录表…），并行会互相 DROP/建表 ⇒ 假失败（实测不加时 dm-query 17 条红、
# dm-migrate 2 条红、dm-pool 的会话数被别的用例抬高）。README「真机」段是同一口径。
#
# 环境变量：
#   TOOLCHAIN       cargo 工具链选择子（默认空 = 本机默认；Windows 侧用 +stable-x86_64-pc-windows-msvc）
#   DM_TEST_DSN     真机连接串；未设时真机阶段自动跳过
#   CARGO            cargo 可执行文件（默认 cargo）
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
LOG_DIR="$REPO/target/gates"
mkdir -p "$LOG_DIR"

TOOLCHAIN="${TOOLCHAIN:-}"
CARGO="${CARGO:-cargo}"
SKIP_REAL=0
SKIP_CROSS=0
SKIP_SOAK=0
COVERAGE=0
ONLY=""
STRICT_SUITE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --skip-real) SKIP_REAL=1 ;;
    --skip-cross) SKIP_CROSS=1 ;;
    --skip-soak) SKIP_SOAK=1 ;;
    --coverage) COVERAGE=1 ;;
    --strict-suite) STRICT_SUITE=1 ;;
    --only) shift; ONLY="${1:-}" ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
  shift
done

CURRENT_LOG=""
CURRENT_NAME=""
IDX=0
declare -a R_NAME R_GROUP R_STATUS R_NOTE

log() { printf '%s\n' "$*" >>"$CURRENT_LOG"; }

# run_cargo <参数…>：日志追加进当前阶段；非零退出码即返回非零。
run_cargo() {
  local out
  out="$("$CARGO" $TOOLCHAIN "$@" 2>&1)"
  local code=$?
  printf '%s\n' "$out" >>"$CURRENT_LOG"
  if [ $code -ne 0 ]; then
    echo "  $CARGO $TOOLCHAIN $*" >&2
    printf '%s\n' "$out" | tail -n 40 | sed 's/^/  | /' >&2
    return $code
  fi
  return 0
}

# run_cargo_raw <参数…>：同 run_cargo，但把输出留在 CURRENT_RAW 供调用者自行判定。
CURRENT_RAW=""
run_cargo_raw() {
  CURRENT_RAW="$("$CARGO" $TOOLCHAIN "$@" 2>&1)"
  local code=$?
  printf '%s\n' "$CURRENT_RAW" >>"$CURRENT_LOG"
  return $code
}

target_installed() {
  # ⚠️ 必须问**同一个**工具链：`rustup target list --installed` 默认只看 default 工具链，
  # 而这里跑的可能是 `TOOLCHAIN=+stable-…`（实测踩过：交叉阶段被误判成 SKIP）。
  if [ -n "$TOOLCHAIN" ]; then
    rustup target list --installed --toolchain "${TOOLCHAIN#+}" 2>/dev/null | grep -qx "$1"
  else
    rustup target list --installed 2>/dev/null | grep -qx "$1"
  fi
}

stage() {  # stage <名字> <分组> <函数名>
  local name="$1" group="$2" fn="$3"
  IDX=$((IDX + 1))
  if [ -n "$ONLY" ]; then
    case ",$ONLY," in *",$name,"*) ;; *) return 0 ;; esac
  fi
  case "$group" in
    cross) [ "$SKIP_CROSS" = 1 ] && return 0 ;;
    real)  { [ "$SKIP_REAL" = 1 ] || [ -z "${DM_TEST_DSN:-}" ]; } && return 0 ;;
  esac
  CURRENT_NAME="$name"
  CURRENT_LOG="$LOG_DIR/$(printf '%02d' "$IDX")-$name.log"
  printf '# cargo 阶段 %s（TOOLCHAIN=%s）\n' "$name" "$TOOLCHAIN" >"$CURRENT_LOG"
  echo
  echo "=== [$IDX] $name ==="

  local start=$SECONDS status="PASS" note="" rc=0
  "$fn" || rc=$?
  if [ "$rc" = 0 ]; then
    status="PASS"
  elif [ "$rc" = 3 ]; then
    # 约定：阶段自己判定「本机不适用」（交叉目标没装、--skip-soak 等）时返回 3 ⇒ 记为 SKIP，
    # 不计入失败（与 `scripts/gates.ps1` 的 `SKIP:` 约定一致）。
    status="SKIP"
    note="阶段主动跳过"
  else
    status="FAIL"
    note="退出码 $rc"
  fi
  local elapsed=$((SECONDS - start))
  if [ "$status" = "PASS" ]; then
    echo "  PASS (${elapsed}s) → $CURRENT_LOG"
  else
    echo "  FAIL: $note" >&2
  fi
  R_NAME+=("$name"); R_GROUP+=("$group"); R_STATUS+=("$status"); R_NOTE+=("$note")
}

# ---------------- 离线组 ----------------

s_deps_l1() {
  local tree
  tree="$($CARGO $TOOLCHAIN tree -p dm-core 2>&1)"
  local code=$?
  printf '%s\n' "$tree" >>"$CURRENT_LOG"
  [ $code -ne 0 ] && return $code
  # §10.5 `deps` 行：L1 运行时无关（dm-core 不得依赖 tokio / odbc）。
  if printf '%s\n' "$tree" | grep -Eq '(^|[[:space:]])(tokio|odbc)([[:space:]]|$)'; then
    printf '%s\n' "$tree" | grep -E '(^|[[:space:]])(tokio|odbc)([[:space:]]|$)' | head -5 | sed 's/^/  | /' >&2
    return 1
  fi
  echo "  dm-core 依赖树无 tokio / odbc ✓"
}

s_feature_matrix() {
  # §10.5「Feature 矩阵」：这些组合必须编译通过。
  run_cargo check --workspace --all-targets || return $?
  run_cargo check --workspace --all-targets --no-default-features || return $?
  run_cargo check --workspace --all-targets --all-features || return $?
  run_cargo check -p dm-core --all-targets --no-default-features --features decimal || return $?
  run_cargo check -p dm-core --all-targets --no-default-features --features chrono || return $?
  run_cargo check -p dm-query --all-targets --no-default-features --features decimal || return $?
  run_cargo check -p dm-query --all-targets --no-default-features --features chrono || return $?
  run_cargo check -p dm-pool --all-targets --features rw-split || return $?
}

s_fmt() { run_cargo fmt --all -- --check; }
s_clippy() { run_cargo clippy --workspace --all-targets --all-features -- -D warnings; }
s_unit_l1() { run_cargo test --workspace --lib --all-features; }
s_l2_contract() { run_cargo test --workspace --all-features --exclude toasty-driver-dameng; }

s_doc_links() {
  RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links' \
    run_cargo doc --workspace --all-features --no-deps
}

s_doctest() { run_cargo test --workspace --doc --all-features; }

s_golden_dameng() {
  # fork 的 golden 必须带 `--all-features`：jiff / rust_decimal / bigdecimal 三条被 cfg 门控。
  run_cargo test -p toasty-sql --test serialize_dameng --all-features
}

s_toasty_offline() { run_cargo test -p toasty-driver-dameng; }

# Round 41 起 `dpi` 是默认特性，`dm-odbc` 是回退后端 ⇒ 必须有一条**关掉 dpi** 的编译门禁，
# 否则回退路径会在没人察觉时烂掉（Round 41 就是这样发现 `DefaultBackend` 被写死的地方）。
s_offline_odbc_backend() {
  run_cargo check -p dm-query --no-default-features --features "chrono decimal" --all-targets &&
    run_cargo check -p dameng --no-default-features --all-targets
}

# ---------------- 交叉编译组（AC-08 类型级） ----------------

s_cross_aarch64() {
  target_installed aarch64-unknown-linux-gnu || { echo "  SKIP: 未安装 aarch64-unknown-linux-gnu"; return 3; }
  run_cargo check --workspace --all-targets --all-features --target aarch64-unknown-linux-gnu
}

s_cross_x86_64() {
  target_installed x86_64-unknown-linux-gnu || { echo "  SKIP: 未安装 x86_64-unknown-linux-gnu"; return 3; }
  run_cargo check --workspace --all-targets --all-features --target x86_64-unknown-linux-gnu
}

# ---------------- 真机组 ----------------

s_real_official_suite() {
  # 期望结果是**全绿**：Round 41 起默认后端是 `dm-dpi`，原先唯一失败的非 BMP 文本缺陷
  # （`type_primitives::ty_str`，见 docs/toasty-fork.md「真机实测发现」第 16 条）在 DPI 上已不复现。
  # 白名单只为「审计 ODBC 回退后端」保留：手工给本阶段加 `--no-default-features`（= 走 `dm-odbc`）时，
  # 允许那 1 条已知缺陷被放行。`--strict-suite` 关掉白名单，要求全绿（审计/发布用）。
  # ⚠️ `--test-threads=1` 是必须的：本机达梦是开发版许可（`v$license` 的
  #   `AUTHORIZED_USER_NUMBER = 1`），并行测试进程会让后起的会话变只读并报
  #   `-6540 … only do readonly`（Round 41 实测：并行时 15 条假失败，串行 851/0）。
  run_cargo_raw test -p toasty-driver-dameng --features real --test driver_suite -- --test-threads=1
  local code=$?
  local line passed failed names
  line="$(printf '%s\n' "$CURRENT_RAW" | grep -E '[0-9]+ passed; [0-9]+ failed' | tail -n 1)"
  if [ -z "$line" ]; then
    echo "  官方套件没有输出 test result 行（退出码 $code）" >&2
    return 1
  fi
  passed="$(printf '%s' "$line" | sed -E 's/.* ([0-9]+) passed; ([0-9]+) failed.*/\1/')"
  failed="$(printf '%s' "$line" | sed -E 's/.* ([0-9]+) passed; ([0-9]+) failed.*/\2/')"
  names="$(printf '%s\n' "$CURRENT_RAW" | sed -nE 's/^test (.+) \.\.\. FAILED$/\1/p' | sort -u | tr '\n' ',')"
  echo "  官方套件：${passed} passed / ${failed} failed；失败：${names:-无}"
  if [ "$STRICT_SUITE" = 1 ]; then
    [ "$code" = 0 ] && return 0
    echo "  官方一致性套件未全绿（--strict-suite）：${passed} passed / ${failed} failed" >&2
    return 1
  fi
  [ "$code" = 0 ] && return 0
  if [ "$failed" = 1 ] && [ "$names" = "type_primitives::ty_str," ]; then
    echo "  PASS（放行 1 条已知缺陷：type_primitives::ty_str —— 非 BMP 文本，见 docs/toasty-fork.md 第 16 条）"
    return 0
  fi
  echo "  官方套件的失败集合与已知缺陷白名单不符（失败：${names:-无}）" >&2
  return 1
}
s_real_toasty() {
  run_cargo test -p toasty-driver-dameng --features real --test toasty_real -- --include-ignored --test-threads=1
}
# ⚠️ 下面几个真机阶段的 `--test-threads=1` 是必须的（README「真机」段同款口径）：
# 用例共用固定表名（DM_Q_REAL_SEED、迁移记录表…），并行会互相 DROP/建表 ⇒ 假失败
# （实测不加时 dm-query 17 条红、dm-migrate 2 条红、dm-pool 的会话数被别的用例抬高）。
s_real_l3_odbc() { run_cargo test -p dm-odbc --test integration -- --include-ignored --test-threads=1; }
s_real_dm_query() { run_cargo test -p dm-query --test query_real -- --include-ignored --test-threads=1; }

# Round 34 起新增的模式级功能测试（DBTESTA / DBTESTB / 小写模式），Round 36/37 扩到 16 条。
s_real_dm_schema() { run_cargo test -p dm-query --test schema_real -- --include-ignored --test-threads=1; }

# Round 41 起默认后端是 `dm-dpi`：直接测驱动后端本身（8 条）。`dm-odbc` 的 74 条由 `real-l3-odbc` 覆盖。
s_real_dpi_backend() { run_cargo test -p dm-dpi --test backend_real -- --include-ignored --test-threads=1; }
s_real_dm_migrate() { run_cargo test -p dm-migrate --test migrate_real -- --include-ignored --test-threads=1; }
s_real_pool_soak() {
  [ "$SKIP_SOAK" = 1 ] && { echo "  SKIP: --skip-soak"; return 3; }
  run_cargo test -p dm-pool --test g4_real -- --include-ignored --test-threads=1 --nocapture
}
s_real_example() { run_cargo run -p toasty-driver-dameng --example toasty_crud; }

s_coverage_ac01() {
  "$CARGO" llvm-cov clean --workspace >>"$CURRENT_LOG" 2>&1 || return $?
  run_cargo llvm-cov -p dm-error -p dm-core -p dm-pool -p dm-query --all-features
}

stage deps-l1         offline s_deps_l1
stage feature-matrix  offline s_feature_matrix
stage fmt             offline s_fmt
stage clippy          offline s_clippy
stage unit-l1         offline s_unit_l1
stage l2-contract     offline s_l2_contract
stage doc-links       offline s_doc_links
stage doctest         offline s_doctest
stage golden-dameng   offline s_golden_dameng
stage toasty-offline  offline s_toasty_offline
stage offline-odbc-backend offline s_offline_odbc_backend
stage cross-aarch64   cross   s_cross_aarch64
stage cross-x86_64    cross   s_cross_x86_64
stage real-official-suite real s_real_official_suite
stage real-toasty         real s_real_toasty
stage real-l3-odbc        real s_real_l3_odbc
stage real-dm-query       real s_real_dm_query
stage real-dm-schema      real s_real_dm_schema
stage real-dpi-backend    real s_real_dpi_backend
stage real-dm-migrate     real s_real_dm_migrate
stage real-pool-soak      real s_real_pool_soak
stage real-example        real s_real_example

if [ "$COVERAGE" = 1 ]; then
  stage coverage-ac01 coverage s_coverage_ac01
fi

echo
echo "=== 门禁汇总 ==="
fails=0
passes=0
# 用 `${#R_NAME[@]}` 而不是直接展开：bash < 4.4 在 `set -u` 下展开**空**数组会报 unbound
# （麒麟/国产系统上可能撞到老 bash）。
if [ "${#R_NAME[@]}" -gt 0 ]; then
  for i in "${!R_NAME[@]}"; do
    printf '%-22s %-9s %s\n' "${R_NAME[$i]}" "${R_GROUP[$i]}" "${R_STATUS[$i]}"
    [ "${R_STATUS[$i]}" = "PASS" ] && passes=$((passes + 1))
    [ "${R_STATUS[$i]}" = "FAIL" ] && { fails=$((fails + 1)); echo "FAIL  ${R_NAME[$i]}: ${R_NOTE[$i]}" >&2; }
    [ "${R_STATUS[$i]}" = "SKIP" ] && echo "SKIP  ${R_NAME[$i]}: ${R_NOTE[$i]}"
  done
fi
echo
echo "通过 $passes / 失败 $fails / 跳过 $((${#R_NAME[@]} - passes - fails))"
if [ "$fails" -gt 0 ]; then
  echo "门禁未通过。" >&2
  exit 1
fi
echo "门禁全部通过。"
exit 0
