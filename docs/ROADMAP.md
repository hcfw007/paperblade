# 路线图

现状快照,以及通往可用 v0.1 的计划路径。各部分如何衔接见
[ARCHITECTURE.md](./ARCHITECTURE.md)。

## 引擎策略

结构性操作(合并、拆分、加/解密)用纯 Rust 库 **lopdf** 内嵌实现,无外部依赖;
重渲染类操作(压缩、栅格化)预计在需要时再引入打包的 CLI 引擎(Ghostscript)
作为 sidecar。

## 当前进度

合并已端到端打通:lopdf 命令 + dialog 选文件 + 合并页面,Rust 侧有合并算法的
集成测试。其余工具仍是脚手架。

| 工具 | 页面 | 命令 | 引擎 | 状态 |
|------|------|------|------|------|
| 合并 Merge | ✅ | `merge_pdfs` | lopdf | ✅ 已完成 |
| 拆分 Split | — | — | lopdf | ❌ 下一步 |
| 加密/解密 Encrypt | — | — | lopdf | ❌ 计划中 |
| 压缩 Compress | — | — | ghostscript | ❌ 计划中 |
| 水印 Watermark | — | — | 待定 | ❌ 计划中 |
| 转换 Convert | — | — | ghostscript / 待定 | ❌ 计划中 |

## 里程碑 0 —— 引擎管道(已完成)

第一个纵向切片端到端验证了 前端 → 命令 → lopdf → 落盘文件 这条路径。

- [x] 引入 lopdf,在 `src-tauri/src/merge.rs` 实现合并算法
- [x] 接入 `tauri-plugin-dialog`,授予 `dialog:default` 权限
- [x] 在 `src-tauri/src/lib.rs` 实现 `merge_pdfs` 命令并注册
- [x] 搭建合并页面:选文件、排序、删除、选输出、执行、反馈结果
- [x] 把引擎错误以友好信息呈现到 UI(`Result<_, String>` → 状态栏)
- [x] 合并算法集成测试(两个单页 PDF → 2 页)
- [ ] 在真实 `pnpm tauri dev` 窗口里手动冒烟一次(需图形环境)

完成标准:用户从 UI 合并两个 PDF 并得到一个有效的输出文件。

## 里程碑 1 —— 补齐核心工具

每个都复用里程碑 0 的模式;唯一新增的部分是引擎调用。

- [ ] **拆分 Split** —— 页码范围 / 每 N 页 / 按大小 → lopdf
- [ ] **加密/解密 Encrypt** —— 设置/移除密码 → lopdf
- [ ] **压缩 Compress** —— 质量预设(screen/ebook/printer)→ Ghostscript sidecar

## 里程碑 2 —— 重渲染类工具

- [ ] **水印 Watermark** —— 在每页叠加文字/图片(选定并验证渲染器)
- [ ] **转换 Convert** —— PDF→图片 与 图片→PDF

## 里程碑 3 —— 发布加固

- [ ] 把 `csp: null` 替换为真实的 Content-Security-Policy
- [ ] 为长任务(压缩、转换)加进度事件
- [ ] 为每个工具的正常路径加 E2E 冒烟测试(Playwright)
- [ ] 对 macOS 构建做代码签名 + 公证
- [ ] 跨平台引擎打包(Windows/Linux)

## 新增工具的约定

1. 在 `src/lib/tools.ts` 添加/确认条目(可用前保持 `available: false`)。
2. 创建 `src/routes/<tool>/+page.svelte` —— 选文件、设选项、执行、出结果。
3. 在 Rust 中加一个 `#[tauri::command]`;校验输入;派生引擎。
4. 在 `invoke_handler![]` 注册该命令。
5. 只授予该命令所需的新权限。
6. 置 `available: true` 并冒烟测试整条路径。
