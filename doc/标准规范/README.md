# 标准规范（工具与工件）

**规范正文不在这个目录。** 按「库即源」，正文只存在于 Obsidian 库：

```
../规范仓库/
  系统研发规范/       000–700 十一篇（唯一事实源）
  系统研发附录/       检查报告、两份规划清单
  系统研发规范总览.md  索引与依赖图
```

本目录只放**机检工具、schema、样例与反例**。

## 路径约定

所有脚本从 [tools/spec_paths.py](tools/spec_paths.py) 取路径，默认：

| 常量 | 相对位置 |
| --- | --- |
| `REPO` | 本目录 |
| `VAULT` | `REPO.parent / "规范仓库"` |
| `SPEC_DIR` | `VAULT / "系统研发规范"` |
| `APPENDIX_DIR` | `VAULT / "系统研发附录"` |
| `OVERVIEW` | `VAULT / "系统研发规范总览.md"` |

库根可用环境变量覆盖，换库不用改代码：

```powershell
$env:SPEC_VAULT = "D:\notes\规范仓库"
```

代码里不写绝对路径。工件（`docs/`）与表格类检查都按仓库相对路径解析。

## 命令

```bash
# 改完笔记后，重建「关联」段与总览
python tools/sync_vault.py

# 库内自检：属性、规则唯一性、判定、章节、表格、链接、依赖一致性、总览
python tools/verify_vault.py

# 规范机检：同样的结构检查 + schema 同步校验
python tools/xtask.py spec-check

# 13 个门禁的回归（24 个正反例）
python tools/run_selfcheck.py
```

## 目录

| 路径 | 内容 |
| --- | --- |
| `tools/xtask.py` | 13 个可运行门禁（spec-check / req-lint / testcase-lint / release-lint / artifact-lint / trace / commit-lint / errcodes / dep-graph / contract-test / eval-gate / agent-registry / coverage） |
| `tools/spec_paths.py` | 路径唯一来源 |
| `tools/sync_vault.py` | 库即源：重建关联与总览 |
| `tools/verify_vault.py` | 库内自检 |
| `tools/schemas/` | 6 个 JSON schema（从 700 篇抽取，`sync-schemas` 保持同步） |
| `docs/` | 工件样例：需求卡、用例、发布单、评审、交接包、提案、错误码表、分层、契约、评测 |
| `tests/fixtures/` | 必须被拦的反例，以及依赖图 / 覆盖率夹具 |
| `archive/` | 旧版文档与已废弃的导入脚本 |

## 为什么 archived

`archive/tools/` 里的两个脚本（`import_to_obsidian.py`、`verify_obsidian_import.py`）属于「文件系统为源」模式，路径写死在该模式上。切换为库即源后，它们的功能由 `sync_vault.py` 与 `verify_vault.py` 接替，保留仅作历史参考。
