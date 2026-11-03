/**
 * DSH Companion —— Obsidian 侧的薄客户端。
 *
 * 它做三件事，**只做这三件**：
 *   1. 用 Obsidian 自己的 `metadataCache` 算出**整库图谱索引**，推给 DSH 的入站桥；
 *   2. 让用户把"当前笔记 / 当前选区"发给 DSH（命令、ribbon、`obsidian://dsh` 协议）；
 *   3. 如实显示连接状态。
 *
 * **它从不写任何笔记**（没有 `vault.modify/create/delete` 调用）。DSH 侧才是唯一写入方，
 * 且写入永远经过门禁与回读校验。这个分工是刻意的：两个写入方 = 两套并发控制 = 迟早互相覆盖。
 *
 * ## 为什么是手写的单个 main.js（而不是 TypeScript + esbuild）
 *
 * 设计初稿写的是"TypeScript + esbuild 打包成单文件 main.js"。实际交付是**手写的 JS 单文件**：
 *   - Obsidian 只加载 `main.js`，它必须是 CommonJS-ish 的 `module.exports = class`；
 *   - `require('obsidian')` **只在 Obsidian 运行时存在**，任何打包器都无法把它的类型或实现解析进来；
 *   - 本仓库没有构建链（离线），而加一条 TypeScript + esbuild 链只为了这一个文件，
 *     换来的是"多一份产物必须与源码同步"的风险。
 * 代价是没有类型检查。对策：**用测试做结构断言**（`test/companion.test.js` 会检查本文件
 * 必须/禁止出现哪些 API 调用），这比类型更能守住"从不写笔记"这类纪律。
 *
 * ## 已核验的 Obsidian 侧 API 事实（决定了下面的写法）
 *   - `app.metadataCache.getBacklinksForFile` **不是公开 API** → 反链必须由 `resolvedLinks` 自己反推；
 *   - `resolvedLinks` / `unresolvedLinks` 是 `{源路径: {目标路径: 次数}}`；
 *   - `getFileCache(file)` 给出 `headings` / `blocks` / `frontmatter` / `tags` / `links`；
 *   - `requestUrl` 是 Obsidian 提供的（绕过 CORS），**必须自己设超时**，并且要传 `throw:false`
 *     才能拿到非 2xx 的响应而不是抛错；
 *   - `app.secretStorage.getSecret/setSecret` 在支持它的版本上可用（本插件做了能力探测）。
 */

'use strict'

const {
  Plugin,
  Notice,
  PluginSettingTab,
  Setting,
  requestUrl,
} = require('obsidian')

/** 与 DSH 侧共用的契约版本（两边不一致就必须拒收，见 bridge.js）。 */
const SCHEMA_VERSION = 1

const DEFAULTS = {
  bridgeUrl: 'http://127.0.0.1:19387/obsidian-bridge',
  debounceMs: 4000,
  maxFiles: 2000,
  pushOnStart: true,
  pingIntervalMs: 60_000,
  requestTimeoutMs: 15_000,
  includeUnresolved: true,
}

const STATUS = {
  off: { text: 'DSH: 未配置', title: '还没有填桥地址或令牌' },
  ok: { text: 'DSH: 已连接', title: '最近一次推送成功' },
  pushing: { text: 'DSH: 推送中…', title: '正在推送图谱索引' },
  fail: { text: 'DSH: 连接失败', title: '推送失败，鼠标悬停看详情' },
  idle: { text: 'DSH: 待推送', title: '有改动尚未推送' },
}

module.exports = class DshCompanionPlugin extends Plugin {
  async onload () {
    this.settings = Object.assign({}, DEFAULTS, (await this.loadData()) || {})
    // ⚠️ 令牌**优先从 secretStorage 读**（不进 data.json，不进同步）
    this.token = await this.readToken()
    this.timer = null
    this.pingTimer = null
    this.lastError = ''
    this.index = null

    this.addSettingTab(new DshCompanionSettingTab(this.app, this))

    this.statusEl = this.addStatusBarItem()
    this.setStatus(this.token && this.settings.bridgeUrl ? 'idle' : 'off')

    this.addRibbonIcon('git-fork', 'DSH：推送图谱索引', () => { this.pushNow() })

    this.addCommand({
      id: 'push-graph-index',
      name: '推送图谱索引到 DSH',
      callback: () => { this.pushNow() },
    })
    this.addCommand({
      id: 'send-current-note',
      name: '把当前笔记路径发给 DSH',
      checkCallback: (checking) => {
        const file = this.app.workspace.getActiveFile()
        if (!file) return false
        if (!checking) this.sendSelection({ path: file.path, kind: 'note' })
        return true
      },
    })
    this.addCommand({
      id: 'send-current-selection',
      name: '把当前选区发给 DSH',
      editorCallback: (editor, view) => {
        const selected = editor.getSelection()
        if (!selected) {
          new Notice('没有选中内容')
          return
        }
        this.sendSelection({
          path: view?.file?.path,
          kind: 'selection',
          // 选区是**用户主动发**的，所以这是唯一会离开库的正文；仍然限制长度
          text: selected.slice(0, 20_000),
        })
      },
    })
    this.addCommand({
      id: 'check-bridge',
      name: '检查 DSH 桥连接',
      callback: () => { this.ping({ announce: true }) },
    })

    // `obsidian://dsh?action=push` / `?action=note&path=…`
    this.registerObsidianProtocolHandler('dsh', (params) => {
      const action = params?.action
      if (action === 'push') { this.pushNow(); return }
      if (action === 'note' && params.path) { this.sendSelection({ path: params.path, kind: 'note' }); return }
      new Notice('DSH：未知的 action')
    })

    // ── 变更监听：防抖后重算并推送 ──
    // 刻意用 metadataCache 的 changed（frontmatter/标签变化时它才准）
    // 加上 vault 的 create/delete/rename（否则图谱里的节点会一直陈旧）
    const schedule = () => { this.schedulePush() }
    this.registerEvent(this.app.vault.on('create', schedule))
    this.registerEvent(this.app.vault.on('delete', schedule))
    this.registerEvent(this.app.vault.on('rename', schedule))
    this.registerEvent(this.app.metadataCache.on('changed', schedule))
    this.registerEvent(this.app.metadataCache.on('resolved', schedule))

    this.app.workspace.onLayoutReady(() => {
      if (this.settings.pushOnStart) this.pushNow()
      if (this.settings.pingIntervalMs > 0) {
        this.pingTimer = window.setInterval(() => { this.ping({ announce: false }) }, this.settings.pingIntervalMs)
        this.registerInterval(this.pingTimer)
      }
    })
  }

  onunload () {
    if (this.timer) window.clearTimeout(this.timer)
    if (this.pingTimer) window.clearInterval(this.pingTimer)
    // 刻意不做任何"清理写入"：本插件从不改库
  }

  /* ────────────────────────── 令牌 ────────────────────────── */

  /**
   * 读令牌：优先 `app.secretStorage`（不进 data.json → 不会被同步到别的机器）。
   * 老版本没有 secretStorage → 退回 data.json 里的明文，并在设置页**如实警告**。
   */
  async readToken () {
    const secretStorage = this.app?.secretStorage
    if (secretStorage?.getSecret) {
      try {
        const value = await secretStorage.getSecret(`dsh-companion-token`)
        if (value) return value
      } catch { /* 不支持就退回去 */ }
    }
    return this.settings.token || ''
  }

  async writeToken (token) {
    // ⚠️ 必须同时更新**内存**里的 this.token：否则存储与内存会分叉，
    // 调用方看起来"存成功了"但插件还在用旧令牌（测试抓出来的真实缺陷）。
    this.token = token
    const secretStorage = this.app?.secretStorage
    if (secretStorage?.setSecret) {
      await secretStorage.setSecret(`dsh-companion-token`, token)
      return true
    }
    await this.saveSettings({ token })
    return false
  }

  hasSecretStorage () {
    return Boolean(this.app?.secretStorage?.setSecret)
  }

  /* ────────────────────────── 状态栏 ────────────────────────── */

  setStatus (key, detail) {
    const entry = STATUS[key] || STATUS.off
    if (!this.statusEl) return
    this.statusEl.setText(entry.text)
    this.statusEl.setAttribute('aria-label', detail ? `${entry.title}\n${detail}` : entry.title)
  }

  /* ────────────────────────── 索引 ────────────────────────── */

  /**
   * 用 metadataCache 构造图谱快照。
   *
   * **只发结构，不发正文、不发属性值**（只发键名）—— DSH 侧会**拒收**带 content/frontmatter 的快照，
   * 这是两边共同的契约，不是单方面的自觉。
   */
  buildSnapshot () {
    const cache = this.app.metadataCache
    const resolved = cache.resolvedLinks || {}
    const unresolved = cache.unresolvedLinks || {}

    // 反链：getBacklinksForFile **不是公开 API** → 从 resolvedLinks 自己反推
    const backlinks = new Map()
    for (const [from, targets] of Object.entries(resolved)) {
      for (const to of Object.keys(targets || {})) {
        if (!backlinks.has(to)) backlinks.set(to, [])
        backlinks.get(to).push(from)
      }
    }

    const all = this.app.vault.getMarkdownFiles()
    const files = all.slice(0, this.settings.maxFiles)
    const partial = all.length > files.length

    const entries = files.map((file) => {
      const fileCache = cache.getFileCache(file) || {}
      const links = Object.keys(resolved[file.path] || {})
      return {
        path: file.path,
        mtime: file.stat?.mtime ?? 0,
        size: file.stat?.size ?? 0,
        // 标签：去掉前导 '#'（Obsidian 的 tags 带 '#'）
        tags: (fileCache.tags || []).map((t) => String(t.tag || '').replace(/^#/, '')).filter(Boolean),
        headings: (fileCache.headings || []).map((hd) => ({
          heading: hd.heading,
          level: hd.level,
        })),
        // 块 id 是 `within` 的另一种定位方式，值得带上
        blocks: Object.keys(fileCache.blocks || {}),
        // ⚠️ **只发键名**，绝不发属性值（值里常有私密内容）
        frontmatterKeys: Object.keys(fileCache.frontmatter || {}),
        links,
        backlinks: backlinks.get(file.path) || [],
      }
    })

    // 未解析链接：摊平成 { from, target }，便于 DSH 直接回答"哪里写了不存在的笔记"
    const unresolvedList = []
    if (this.settings.includeUnresolved) {
      for (const [from, targets] of Object.entries(unresolved)) {
        for (const target of Object.keys(targets || {})) unresolvedList.push({ from, target })
      }
    }

    const orphans = entries
      .filter((entry) => entry.links.length === 0 && entry.backlinks.length === 0)
      .map((entry) => entry.path)

    return {
      schemaVersion: SCHEMA_VERSION,
      // 库身份：DSH 侧用 obsidianVaultRef 与它比对（插件没有任何发现机制，只能靠这个）
      vaultRef: this.app.vault.getName(),
      generatedAt: new Date().toISOString(),
      partial,
      totalFiles: all.length,
      files: entries,
      unresolved: unresolvedList,
      orphans,
    }
  }

  /* ────────────────────────── 推送 ────────────────────────── */

  schedulePush () {
    if (this.timer) window.clearTimeout(this.timer)
    this.setStatus('idle')
    this.timer = window.setTimeout(() => { this.pushNow() }, Math.max(0, this.settings.debounceMs))
  }

  async pushNow () {
    if (!this.settings.bridgeUrl || !this.token) {
      this.setStatus('off')
      return false
    }
    this.setStatus('pushing')
    try {
      const snapshot = this.buildSnapshot()
      this.index = snapshot
      const response = await this.request('snapshot', snapshot)
      if (response.ok) {
        this.lastError = ''
        this.setStatus('ok', `${response.files} 篇${snapshot.partial ? '（已截断）' : ''}`)
        return true
      }
      this.lastError = response.message || `HTTP ${response.status}`
      this.setStatus('fail', this.lastError)
      return false
    } catch (error) {
      this.lastError = String(error?.message || error)
      this.setStatus('fail', this.lastError)
      return false
    }
  }

  /**
   * ping：顺便实现"DSH 重启后自愈" —— 对面说 needSnapshot 就立刻补推一次。
   * 这是本插件唯一会自动重推的时机（其余都靠变更事件与用户手动）。
   */
  async ping ({ announce = false } = {}) {
    if (!this.settings.bridgeUrl || !this.token) {
      if (announce) new Notice('DSH：还没有配置桥地址或令牌')
      this.setStatus('off')
      return false
    }
    try {
      const response = await this.request('ping', {})
      if (!response.ok) {
        this.lastError = response.message || `HTTP ${response.status}`
        this.setStatus('fail', this.lastError)
        if (announce) new Notice(`DSH 桥：${this.lastError}`)
        return false
      }
      if (response.needSnapshot) {
        if (announce) new Notice('DSH 桥：对面还没有索引，正在补推…')
        await this.pushNow()
      } else {
        this.setStatus('ok')
        if (announce) new Notice(`DSH 桥：正常（对面有 ${response.lastPushFiles ?? '?'} 篇）`)
      }
      return true
    } catch (error) {
      this.lastError = String(error?.message || error)
      this.setStatus('fail', this.lastError)
      if (announce) new Notice(`DSH 桥连接失败：${this.lastError}`)
      return false
    }
  }

  /**
   * 发一个请求。
   *
   * 两个必须做的动作：
   *   - `throw: false`：obsidian 的 requestUrl 在非 2xx 时**默认抛错**，那样就拿不到
   *     对面精心写的错误体（我们靠它区分"令牌错"与"没配令牌"）；
   *   - **自己设超时**：requestUrl 没有超时参数，桥可能因为 DSH 没启动而一直挂着。
   */
  async request (subPath, payload) {
    const url = `${String(this.settings.bridgeUrl).replace(/\/+$/, '')}/${subPath}`
    const timeoutMs = Math.max(1000, Number(this.settings.requestTimeoutMs) || 15_000)

    const call = requestUrl({
      url,
      method: 'POST',
      throw: false,
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${this.token}`,
      },
      body: JSON.stringify(payload),
    })

    let timer
    const timeout = new Promise((_, reject) => {
      timer = window.setTimeout(() => reject(new Error(`请求超时（${timeoutMs} ms）：DSH 可能没在运行`)), timeoutMs)
    })

    let raw
    try {
      raw = await Promise.race([call, timeout])
    } finally {
      if (timer) window.clearTimeout(timer)
    }

    let body = {}
    try {
      body = raw.json ?? JSON.parse(raw.text || '{}')
    } catch {
      body = { ok: false, message: `响应不是 JSON（HTTP ${raw.status}）` }
    }
    return { status: raw.status, ...body }
  }

  /* ────────────────────────── 发送路径 / 选区 ────────────────────────── */

  /**
   * 把"当前笔记路径"或"用户选中的文本"发给 DSH。
   *
   * 复用同一个 `/snapshot` 端点会污染契约，所以这里走一个**独立的动作**：
   * 它不作为图谱索引入库，只是通过通知转达（DSH 侧目前只收了快照与 ping；
   * 这条路径是**给未来的交互式桥预留的**，现在会得到一个明确的 404 并如实提示用户）。
   */
  async sendSelection (payload) {
    if (!this.settings.bridgeUrl || !this.token) {
      await navigator.clipboard?.writeText?.(payload.path || '')
      new Notice('DSH 未配置：路径已复制到剪贴板')
      return
    }
    try {
      const response = await this.request('selection', { schemaVersion: SCHEMA_VERSION, ...payload })
      if (response.ok) {
        new Notice(`已发送到 DSH：${payload.path || '（无路径）'}`)
      } else {
        // 如实说明：这条通道现在还没被 DSH 接收
        await navigator.clipboard?.writeText?.(payload.text || payload.path || '')
        new Notice(`DSH 桥未接受这个动作（${response.error || response.status}）；已复制到剪贴板`)
      }
    } catch (error) {
      await navigator.clipboard?.writeText?.(payload.text || payload.path || '')
      new Notice(`发送失败（${error?.message || error}）；已复制到剪贴板`)
    }
  }

  async saveSettings (patch) {
    this.settings = Object.assign({}, this.settings, patch)
    await this.saveData(this.settings)
  }
}

/* ────────────────────────── 设置页 ────────────────────────── */

class DshCompanionSettingTab extends PluginSettingTab {
  constructor (app, plugin) {
    super(app, plugin)
    this.plugin = plugin
  }

  display () {
    const { containerEl } = this
    containerEl.empty()

    containerEl.createEl('h2', { text: 'DSH Companion' })
    containerEl.createEl('p', {
      text: '把本库的图谱索引推送给 DSH。本插件**只读**：它从不修改任何笔记。',
    })

    new Setting(containerEl)
      .setName('DSH 桥地址')
      .setDesc('形如 http://127.0.0.1:19387/obsidian-bridge —— 路径部分要与 DSH 侧配置的 bridgePathPrefix 一致（不要带结尾斜杠）。')
      .addText((text) => text
        .setPlaceholder(DEFAULTS.bridgeUrl)
        .setValue(this.plugin.settings.bridgeUrl)
        .onChange(async (value) => {
          await this.plugin.saveSettings({ bridgeUrl: value.trim() })
        }))

    const tokenSetting = new Setting(containerEl)
      .setName('桥令牌')
      .setDesc(this.plugin.hasSecretStorage()
        ? '令牌保存在 Obsidian 的 secretStorage 里，**不会**写进 data.json，也不会被同步到别的机器。在 DSH 里执行 `/vault bridge token` 生成。'
        : '⚠️ 这个版本的 Obsidian 没有 secretStorage，令牌只能明文存在 data.json 里（会被同步）。请自行评估。')
      .addText((text) => {
        text.inputEl.type = 'password'
        text.setPlaceholder('粘贴 DSH 生成的令牌')
        text.setValue(this.plugin.token || '')
        text.onChange(async (value) => {
          this.plugin.token = value.trim()
          const usedSecret = await this.plugin.writeToken(this.plugin.token)
          if (value.trim() && !usedSecret && this.plugin.hasSecretStorage() === false) {
            // 已经如实警告过，不重复打扰
          }
        })
      })
      .addButton((button) => button
        .setButtonText('测试连接')
        .onClick(async () => { await this.plugin.ping({ announce: true }) }))

    if (this.plugin.lastError) {
      tokenSetting.setDesc(`${tokenSetting.desc}\n\n最近一次错误：${this.plugin.lastError}`)
    }

    new Setting(containerEl)
      .setName('启动时推送')
      .setDesc('Obsidian 打开后自动推一次图谱索引（DSH 重启后靠 ping 也会自动补推）。')
      .addToggle((toggle) => toggle
        .setValue(this.plugin.settings.pushOnStart)
        .onChange(async (value) => { await this.plugin.saveSettings({ pushOnStart: value }) }))

    new Setting(containerEl)
      .setName('防抖（毫秒）')
      .setDesc('库有改动后等多久再重算并推送。默认 4000。太小会让大库一直重算。')
      .addText((text) => text
        .setValue(String(this.plugin.settings.debounceMs))
        .onChange(async (value) => {
          const n = Number(value)
          if (Number.isFinite(n) && n >= 0) await this.plugin.saveSettings({ debounceMs: n })
        }))

    new Setting(containerEl)
      .setName('最多推送篇数')
      .setDesc('超过这个数量只推前 N 篇并在快照里标 partial:true（DSH 会如实告诉你"结果不完整"）。默认 2000。')
      .addText((text) => text
        .setValue(String(this.plugin.settings.maxFiles))
        .onChange(async (value) => {
          const n = Number(value)
          if (Number.isFinite(n) && n > 0) await this.plugin.saveSettings({ maxFiles: Math.floor(n) })
        }))

    new Setting(containerEl)
      .setName('定时 ping（毫秒）')
      .setDesc('定期检查 DSH 是否还在；对面没有索引时会自动补推。设为 0 关闭。默认 60000。')
      .addText((text) => text
        .setValue(String(this.plugin.settings.pingIntervalMs))
        .onChange(async (value) => {
          const n = Number(value)
          if (Number.isFinite(n) && n >= 0) await this.plugin.saveSettings({ pingIntervalMs: n })
        }))

    new Setting(containerEl)
      .setName('请求超时（毫秒）')
      .setDesc('requestUrl 没有超时参数，这里自己设一个，避免 DSH 没运行时一直挂着。默认 15000。')
      .addText((text) => text
        .setValue(String(this.plugin.settings.requestTimeoutMs))
        .onChange(async (value) => {
          const n = Number(value)
          if (Number.isFinite(n) && n >= 1000) await this.plugin.saveSettings({ requestTimeoutMs: Math.floor(n) })
        }))

    new Setting(containerEl)
      .setName('包含未解析链接')
      .setDesc('把"写了但解析不到的 [[链接]]"也推给 DSH（它据此回答"哪里有断链"）。')
      .addToggle((toggle) => toggle
        .setValue(this.plugin.settings.includeUnresolved)
        .onChange(async (value) => { await this.plugin.saveSettings({ includeUnresolved: value }) }))

    new Setting(containerEl)
      .setName('立即推送')
      .addButton((button) => button
        .setButtonText('推送图谱索引')
        .onClick(async () => { await this.plugin.pushNow() }))

    containerEl.createEl('h3', { text: '隐私' })
    containerEl.createEl('p', {
      text: '推送的快照只含**结构**：路径、链接、反链、标签、标题、块 id、属性**键名**。'
        + '不含任何笔记正文，也不含属性值（DSH 侧会拒收带 content/frontmatter 的快照）。'
        + '只有你主动执行"把当前选区发给 DSH"时，选中的文本才会离开本库。',
    })
  }
}
