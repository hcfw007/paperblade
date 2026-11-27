# M2 渲染引擎选型:水印与转换

调研日期:2026-11-26。目的:为里程碑 2 的两个工具(水印 Watermark、转换
Convert:PDF↔图片)选定渲染/处理引擎,解除排期阻塞。纯调研文档,不含功能实现。

> **TL;DR**
> - **水印 → lopdf(纯 Rust,内容流叠加)**:不动原页面内容流,只追加一个水印流,
>   保真度最高,且零新依赖。备选 pdfium。
> - **图片→PDF → printpdf(纯 Rust)**:嵌入式完成,无引擎、无外部进程。
> - **PDF→图片 → pdfium-render + 捆绑 libpdfium**:渲染质量、速度、打包难度、
>   许可(BSD-3-Clause + Apache-2.0)全面优于复用 Ghostscript。备选:复用 gs(零新
>   引擎,但慢、且把 AGPL 履约面扩大到第二个功能)。
> - **MuPDF 不选**:AGPL 且是库内链接,会把整个应用拖进 AGPL;要豁免只能买 Artifex
>   商业许可。
>
> 许可勘误:PDFium 并不是"Apache-3.0"(无此许可证),上游是 **BSD-3-Clause +
> Apache-2.0** 双许可文本并存(见 §2.1 来源)。

## 1. 背景与约束

### 1.1 项目现状

- PaperBlade 是 MIT 许可([LICENSE](../../LICENSE))的本地优先桌面 PDF 工具,
  SvelteKit SPA + Tauri(Rust)外壳。
- 既有引擎分工([ARCHITECTURE.md](../ARCHITECTURE.md)):
  - **结构性操作用纯 Rust 的 lopdf** 内嵌实现——合并、拆分、加/解密,无外部依赖;
  - **重渲染类操作交给 CLI 引擎 Ghostscript**——M1 压缩已落地(`compress.rs`,
    `gs_args()` 纯函数 + 派生进程),但 gs **尚未打包**,当前回退到用户 `PATH` 上的
    `gs`,真正的 sidecar 打包是 M3 前置项。
- 约束([ARCHITECTURE.md](../ARCHITECTURE.md)「目标与约束」):纯本地、**小体积**、
  macOS 优先、一套模式多个工具;错误统一 `Result<T, String>`;长任务发进度事件。

### 1.2 M2 需求拆解(按引擎能力分)

| 功能 | 需要的能力 | 备注 |
|------|-----------|------|
| 水印-文字 | 往**已有 PDF** 每页叠加文字(位置/旋转/透明度),含中文 | 不应重写整份文档 |
| 水印-图片 | 往已有 PDF 每页叠加图片(缩放/透明度) | 图片解码 + Image XObject |
| 转换-图片→PDF | 多图合成多页 PDF(或单图单页) | 纯"写",不读 PDF |
| 转换-PDF→图片 | **PDF 页面栅格化**为 PNG/JPEG(可调 DPI) | 真正的渲染引擎需求 |

关键观察:**图片→PDF 是纯"写"任务,水印是"往现有内容流追加",只有 PDF→图片是
必须引入渲染引擎的重活。** 三个需求的引擎诉求并不相同,可以拆开选型。

### 1.3 评估维度

渲染质量、Rust 生态绑定成熟度、二进制体积、许可合规(本仓库为 MIT 开源桌面
应用)、与现有 Tauri 架构的集成成本、离线可用性。横评表见 §2.6。

## 2. 候选横评

### 2.1 pdfium / pdfium-render(Rust 绑定)

- **项目**:PDFium 是 Chromium 的 PDF 引擎(Google 开源,原 Foxit 开发);
  [ajrcarey/pdfium-render](https://github.com/ajrcarey/pdfium-render) 是其高层
  Rust 封装,crates.io [`pdfium-render`](https://crates.io/crates/pdfium-render)
  0.9.6(2026-11-21 发布),近月下载约 210 万,持续活跃(2026 年内 0.9.0→0.9.6
  多次发布,修复内存安全问题、跟进 pdfium 新版本)。
- **渲染能力**:页面栅格化(任意 DPI、旋转、裁剪)质量高(Chromium 同款渲染);
  文本/图片对象提取;**专门的 `PdfPages::watermark()` API**(逐页回调填充水印对象,
  [docs.rs 文档](https://docs.rs/pdfium-render/latest/pdfium_render/prelude/struct.PdfPages.html)),
  以及创建文本对象、加载外部字体(`PdfFont::load_cid_type2_from_bytes` 等,0.9.6
  起支持)。图片→PDF:可 `create_new_pdf()` + 图片对象,但属"能做"而非"顺手做"。
- **维护活跃度**:高,见上。注意 1.0 之前 API 仍有破坏性变更可能(crate 标注
  pre-1.0)。
- **预编译产物分发**:`pdfium-render` **不含** pdfium 本体,三种绑定方式:运行时
  `dlopen` 系统/随包动态库、静态链接(`static` feature)、WASM。
  [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries)(MIT)
  每周一自动构建全平台动态库(mac-arm64/x64、win、linux、wasm;**明确不做静态
  构建**);另有 [`pdfium-bundled`](https://docs.rs/pdfium-bundled/latest/pdfium_bundled/)
  crate 支持运行时下载缓存(SHA-256 钉死 + SLSA attestation 校验)或
  `bundled` feature 编译期把动态库嵌进二进制(约 +30 MB)。
- **许可**:PDFium 本体 **BSD-3-Clause + Apache-2.0**(上游 LICENSE 文件两段并存:
  [pdfium LICENSE](https://pdfium.googlesource.com/pdfium/+/main/LICENSE));
  pdfium-binaries 构建脚本 MIT;`pdfium-render` 本体 MIT OR Apache-2.0。**与 MIT
  项目兼容,可闭源分发、可上架 Mac App Store,义务仅限保留版权声明与许可文本**
  (可参考第三方应用的履约先例,如 [Filoshi 的开源声明](https://filoshi.com/licenses/pdfium.txt))。
- **与 Tauri 集成**:进程内 `dlopen` 随包动态库即可(无 PATH 逻辑、无进程派生);
  动态库放进 app bundle 需随主程序一起代码签名(公证要求所有二进制签名,见 §5)。
- **离线**:捆绑后完全离线;若走 pdfium-bundled 的运行时下载模式则首启需联网,
  **本项目应使用编译期 `bundled` 或自管动态库打包**,不走运行时下载。
- **体积**:libpdfium 动态库约 20–30 MB(未 strip,随版本浮动,**待实测**);
  纯 Rust 侧绑定代码可忽略。

### 2.2 MuPDF / mupdf-rs

- **项目**:[messense/mupdf-rs](https://github.com/messense/mupdf-rs),crates.io
  [`mupdf`](https://crates.io/crates/mupdf) 0.8.0(2026-06-22)。`mupdf-sys` **从
  源码编译 MuPDF**(首次构建需 C 编译器 + clang/bindgen,耗时数分钟),静态链入。
- **能力**:渲染、文本提取、PDF 创建/编辑、`shape` API 可往页面画文字/矢量
  (水印友好)、SVG 输出。能力上是全才,渲染质量一流。
- **许可**:**AGPL-3.0**(crate 与 MuPDF 本体都是),商业豁免需向 Artifex 购买
  ([Artifex licensing](https://artifex.com/licensing))。mupdf-rs 是**库内链接**
  (静态编译进你的可执行文件),按 FSF/常见法律解读即形成"单一作品",**整个应用
  需以 AGPL 提供源码**——与本项目 MIT 发布冲突;即便改独立进程分发,Artifex 的
  [许可说明](https://help-us.na.smart-integration.ricoh.com/PrintCloud%20Virtual%20Print%20Driver/Help/index6.html)
  也主张"与非 AGPL 应用同介质分发"需商业许可(该主张的法律效力有争议,见 §5)。
- **结论**:**不选**。除非项目整体转 AGPL 或购买商业许可,均不符合当前定位。

### 2.3 Ghostscript(项目已用,M1 压缩)

- **能力**:`-sDEVICE=png16m/jpeg` 栅格化 PDF→图片(可用 `-r` 调 DPI、`%d` 多页
  输出);水印可通过 PostScript `EndPage`/`BeginPage` 钩子或 `pdfmark` 实现,
  是成熟做法([StackOverflow: KenS 关于 EndPage/pdfmark 的回答]
  (https://stackoverflow.com/questions/38693217/how-add-watermark-to-ghostscript-with-command-line)、
  [Transloadit 实践](https://transloadit.com/devtips/automate-pdf-watermarking-with-ghostscript/))。
  文字水印的 CJK 支持依赖 PostScript CID 字体替换,麻烦;透明度需
  `-dALLOWPSTRANSPARENCY` 等私有开关。
- **保真性缺陷(水印场景的硬伤)**:gs 的 `pdfwrite` 是**重写整份 PDF**——数字签名
  失效,表单/注释/标签/色彩可能变化(Transloadit 文档明确警告)。与 lopdf"追加
  内容流"的保真路线相比,对"只想盖个章"的场景副作用过大。
- **许可**:**AGPL-3.0**,Artifex 双许可(商业许可另购)。桌面分发一旦把 gs 打进
  app bundle(M3 计划),即触发 AGPL §6:需随发行物提供 gs 的对应源码(或书面
  要约)、保留版权声明与许可文本;项目自身代码因 gs 是**独立进程**可维持 MIT
  (聚合体观点),但 Artifex 主张"应用含调用 gs 的功能"即需整体 AGPL 或商业许可,
  存在解释分歧(§5)。这正是 ROADMAP 已记录的"已知取舍"。
- **复用它做水印/转图 vs 新引擎的利弊**:
  - 利:零新引擎,M3 打包一份 gs 全功能复用;`compress.rs` 的进程派生模式直接照搬。
  - 弊:水印重写文档、丢签名;栅格化速度慢(进程启动 + PS 解释器)、色彩管理
    与透明度渲染瑕疵多于 pdfium;**扩大了 AGPL 依赖的功能覆盖**(合规面上从
    "压缩一个功能"变成"压缩+水印+转换",任何未来闭源/上架路径的谈判筹码更少);
    gs 打包问题(brew 版链 12 个 dylib)至今未解,新功能会被同一个问题卡住。
- **状态**:M1 已落地,`has_compression_engine` 提前报缺失。**打包问题见
  [ROADMAP](../ROADMAP.md)「Ghostscript 打包(M3 前置)」。**

### 2.4 纯 Rust 方案(printpdf / krilla / lopdf / typst)

- **lopdf**([docs.rs](https://docs.rs/lopdf/latest/lopdf/),MIT)——项目已在用。
  `add_page_contents()` 原生支持"往页面追加内容流,已有内容不动",正是水印的
  标准做法(把水印画成独立流追加到 `/Contents` 末尾,后者画在上层)。社区已有
  多个成熟先例:[cap-doc 的 watermark.rs](https://docs.rs/cap-doc/latest/src/cap_doc/pdf/watermark.rs.html)
  (含 CJK 非嵌入字体方案:Latin 用基 14 Helvetica,中文用 Adobe 预置
  STSong-Light + UniGB-UCS2-H,阅读器自带替换字形,不嵌几 MB 字体;并记录了
  `/Contents` 是间接数组、MediaBox 不从 (0,0) 起算、MuPDF 要求 FontDescriptor
  等实测坑)、[pdf-manip](https://docs.rs/pdf-manip/latest/src/pdf_manip/watermark.rs.html)
  (平铺/旋转/透明度)、[pdforg](https://docs.rs/pdforg/latest/src/pdforg/pdf_tools/stamp.rs.html)。
  **能力边界**:文字/图形/图片(Image XObject,需自己解码图片)水印都可行;
  **不能栅格化**;CJK 精确排版靠非嵌入字体的宽度估算,有误差(见 §5)。
- **printpdf**([crates.io](https://crates.io/crates/printpdf) 0.13.0,2026-11-25,
  MIT)——图片→PDF 一等公民:`RawImage::decode_from_bytes`(png/jpeg/…) +
  `add_image` + `Op::UseXobject`,每图一页、缩放/DPI 明确可控;也能读 PDF、抽文本。
  渲染方向只有 `page_to_svg`(**实验性**,输出 SVG 再经 resvg 转位图,保真度不适合
  当 PDF→图片的产品能力)。基于 lopdf,与现有依赖同生态。
- **krilla**([crates.io](https://crates.io/crates/krilla) 0.8.2,MIT OR Apache-2.0)
  ——Typst 的 PDF 导出后端([typst#5420](https://github.com/typst/typst/pull/5420)),
  高质量"写"PDF(字体子集化、透明、渐变、PDF/A),图片嵌入一流;但**只创建、
  不读不改**已有 PDF,水印无用武之地;图片→PDF 比 printpdf 更精但 API 更底层。
- **typst**——排版引擎全家桶,为"文档排版"而生,做图片→PDF 属于杀鸡用牛刀
  (引入完整排版器与字体集);其 PDF 导出底层就是 krilla,需要写能力直接用 krilla。
  许可以仓库 LICENSE 为准(Apache-2.0,**待复核**)。

### 2.5 一句话排除

- **qpdf**(Apache-2.0,结构/线性化/加密,**无渲染**,做不了栅格化与水印图形);
- **poppler**(GPL-2.0+,`pdftoppm` 能栅格化但**链接即传染 GPL**,与 MIT 定位冲突;
  只能以独立进程"聚合"方式用,合规解释同样有分歧,不如 pdfium 干净);
- **veraPDF**(GPL-3.0+ / MPL-2.0 双许可,Java,PDF/**A 校验器**,不是渲染器)。

### 2.6 横评表

图例:✅ 好 / ⚠️ 有保留 / ❌ 不满足。「体积增量」指进入 app bundle 后的增量。

| 维度 | pdfium (pdfium-render) | MuPDF (mupdf-rs) | Ghostscript | lopdf(已有) | printpdf | krilla |
|------|------------------------|------------------|-------------|-------------|----------|--------|
| 渲染质量(栅格化) | ✅ Chromium 同款 | ✅ 一流 | ⚠️ 够用,色彩/透明瑕疵 | ❌ 无渲染 | ⚠️ 仅 SVG(实验) | ❌ 只写不渲染 |
| 水印(改已有 PDF) | ✅ 专设 `watermark()` API,可嵌字体 | ✅ shape API | ⚠️ 能做但重写文档、丢签名 | ✅ 追加内容流,保真最好 | ❌ 读写支持弱 | ❌ 不能改已有 PDF |
| 图片→PDF | ⚠️ 能做,不顺手 | ✅ 能做 | ✅ 能做(重) | ⚠️ 需手搓 XObject | ✅ 一等公民 | ✅ 高质量(底层) |
| Rust 绑定成熟度 | ✅ 0.9.x 活跃,428 万下载 | ⚠️ 0.8.0,更新较慢 | ✅(进程调用,非绑定) | ✅ 项目已用 | ✅ 0.13.0 活跃 | ✅ typst 后端,活跃 |
| 二进制体积增量 | ⚠️ ~20–30 MB 动态库(待实测) | ⚠️ 静态链入 ~15–30 MB(待实测) | ⚠️ sidecar ~15–30 MB(打包未解) | ✅ 0(已在依赖里) | ✅ ~1 MB 级 | ✅ ~几 MB |
| 许可合规(MIT 桌面) | ✅ BSD-3 + Apache-2.0,仅需保留声明 | ❌ AGPL 链接即整体传染 | ⚠️ AGPL,独立进程聚合,履约有义务+解释分歧 | ✅ MIT | ✅ MIT | ✅ MIT OR Apache-2.0 |
| Tauri 集成成本 | ⚠️ 捆绑 dylib + 签名,进程内调用 | ⚠️ 构建期编译 MuPDF,链入主程序 | ⚠️ 进程派生模式已有,但打包未解 | ✅ 直接调库 | ✅ 直接调库 | ✅ 直接调库 |
| 离线可用 | ✅(捆绑后;勿用运行时下载模式) | ✅ | ✅(打包后;现依赖用户 PATH) | ✅ | ✅ | ✅ |

补充读数:

- **许可合规行的细节**:pdfium 只需随发行物保留版权声明 + 许可文本(无源码提供
  义务);gs/mupdf 的 AGPL 在**分发**时要求提供"对应源码"(gs 因是独立进程,义务
  对象是 gs 自身及其修改,项目代码可维持 MIT——但见 §5 的解释分歧);mupdf 因
  静态链接连解释空间都没有。
- **集成成本行的细节**:pdfium 的 dylib 是自包含的(仅依赖系统库),直接进
  bundle;brew 的 gs 链着 12 个 homebrew dylib,静态化或重写路径才能分发,这个
  问题 M1 起就没解决,水印/转换若走 gs 会一并被卡。
- **水印行的细节**:lopdf 路线是"追加内容流"(原内容字节不动);pdfium 路线保存
  时会重新序列化文档(多数对象保留,但属"编辑器保存"语义);gs 是最差的一档
  (pdfwrite 全量重排)。

## 3. 分功能推荐

允许不同功能用不同引擎。按 §1.2 的能力拆解,三个子需求各自给出推荐。

### 3.1 水印 —— 首选 lopdf(纯 Rust 内容流叠加)

**做法**:把水印画成一个独立内容流,追加到每页 `/Contents` 数组末尾(后画者在
上层),字体走非嵌入方案(Latin 用基 14 Helvetica;中文用 Adobe 预置
STSong-Light + UniGB-UCS2-H),透明度走 ExtGState(`ca`/`CA`),旋转/定位用
`cm`/`Tm` 矩阵。社区已有可对照的完整实现([cap-doc watermark.rs]
(https://docs.rs/cap-doc/latest/src/cap_doc/pdf/watermark.rs.html))。

**理由**:

1. **保真度最高**——原页面内容流一个字节都不碰,书签/表单/签名等结构不受影响
   (数字签名在任何"改内容"的方案下都会失效,但 lopdf 路线的副作用面最小);
2. **零新依赖**——lopdf 已是现有四个工具的引擎,正好落在
   [ARCHITECTURE.md](../ARCHITECTURE.md)"结构性操作用纯 Rust"的既有分工里;
3. **零体积增量、完全离线**,不等 M3 打包工作,可以立刻开工;
4. 图片水印同样走 lopdf(Image XObject + `Do`),配合 `image` crate 解码即可
   (或复用 printpdf 的 `RawImage` 解码后取原始数据,两者同生态)。

**已知短板**(对应风险 §5-R3):非嵌入 CJK 字体的实际字宽由阅读器替换字体决定,
水印的宽度估算只能靠经验值(先例用 1.25 系数兜底),居中/缩放精度有偏差;若
产品要求像素级排版或自定义字体,升级到备选方案。

**备选:pdfium**——`PdfPages::watermark()` 是为这个场景特设的 API,文本测量精确、
可加载任意 TTF/OTF(含中文)并嵌入,图片/透明/旋转原生支持。代价是提前引入
pdfium 动态库。**决策触发条件**:若 M2a 实测 lopdf 方案在 Adobe Reader / PDFKit /
Chrome / MuPDF 四个阅读器下的 CJK 水印排版不可接受,再切 pdfium(此时转换功能
大概率已经把 pdfium 引进来了,切换成本低)。

**不选 gs 做水印**:pdfwrite 重写整份文档,签名失效、表单/注释/色彩可能被改动
([Transloadit 明确警告](https://transloadit.com/devtips/automate-pdf-watermarking-with-ghostscript/)),
"盖个章"不该有这种副作用;且 PostScript 的 CJK/透明度处理繁琐。

### 3.2 转换·图片→PDF —— 首选 printpdf(纯 Rust)

**做法**:每张图一页,`RawImage::decode_from_bytes` → `add_image` →
`Op::UseXobject` 摆放;页面尺寸按图片 DPI 折算或固定 A4 居中(产品选项)。

**理由**:printpdf 对这个场景是一等公民(多格式解码、DPI 明确、release 构建自动
压缩图像数据),MIT 许可,基于 lopdf 与现有依赖同生态,零引擎、零体积、完全离线。
krilla 质量更精(字体/渐变/压缩),但 API 面向"布局好的内容转 PDF",对"几张图
拼一本"偏底层——除非后续对输出 PDF/A 或体积有硬要求,不值得换。

**备选:krilla**;**亦可做:pdfium**(若已引入)、**lopdf 手搓**(不推荐,重复造轮子)。

### 3.3 转换·PDF→图片 —— 首选 pdfium-render(捆绑 libpdfium)

**做法**:`Pdfium::bind_to_library`(随包动态库)→ 逐页
`PdfRenderConfig` 设 DPI/尺寸/背景 → 渲染位图 → PNG/JPEG 编码落盘。

**理由**:

1. **渲染质量**是四个候选里最好的一档(Chromium 同款),复杂 PDF(透明、
   渐变、CMYK、注释)的保真显著好于 gs 的 png16m/jpeg 设备;
2. **速度**:进程内渲染,批量转几百页比 gs 快一个量级(gs 每次是完整 PS 解释
   器启动)。
3. **许可干净**:BSD-3 + Apache-2.0,捆绑无源码义务、无上架障碍——把
   "AGPL 引擎"的覆盖面控制在压缩一个功能,不让合规债扩大;
4. **打包可行性**:pdfium-binaries 的动态库自包含,进 bundle 即用;这比 gs 的
   打包问题(brew dylib 纠缠)先落地的概率大得多。

**备选:复用 gs**(`-sDEVICE=png16m -r150 -dSAFER`)——零新引擎、复用 `compress.rs`
的派生进程模式,M3 打包一份 gs 三处用。缺点:慢、色彩/透明渲染瑕疵、AGPL 依赖
面扩大、打包问题依旧。**决策触发条件**:若 M3 前置项"gs 打包"最终方案落地成本
远低于 pdfium 捆绑(例如已解决静态化),且对转换速度/质量要求不高,可降级用 gs
过渡,pdfium 留给 M3 后替换。

**排除 macOS PDFKit/CoreGraphics 的说明**:系统框架零体积零许可问题,渲染质量好,
且测试已在用 PDFKit 做外部验证;但仅限 macOS,M3 跨平台(Windows/Linux)需再选
一套引擎,等于把选型债推迟而非消除。可作为"仅 macOS 内测版"的临时方案,
**待验证**其通过 Rust(core-foundation-sys 一类)调用的集成成本。

## 4. 决策建议

| 功能 | 首选 | 备选 | 不选(理由) |
|------|------|------|------------|
| 水印(文字/图片) | **lopdf 内容流叠加**(纯 Rust) | pdfium(排版精度不达标时) | gs(重写文档、丢签名);MuPDF(AGPL 链接传染) |
| 转换·图片→PDF | **printpdf**(纯 Rust) | krilla(质量/体积有硬要求时) | gs(杀鸡用牛刀);MuPDF(同上) |
| 转换·PDF→图片 | **pdfium-render + 捆绑 libpdfium** | 复用 gs(打包成本压倒性时) | printpdf(仅 SVG 实验品,保真不够);poppler(GPL) |

决策要点:

1. **保持既有分工哲学**:结构性/纯"写"任务走纯 Rust(lopdf、printpdf),只有
   真正的栅格化引入渲染引擎(pdfium)。M2 实际只新增一个原生依赖(pdfium 动态库)
   加一个纯 Rust crate(printpdf)。
2. **许可账**:三个首选里两个是 MIT、一个是 BSD-3/Apache-2.0,均无源码提供义务,
   不影响项目 MIT 定位与潜在上架;AGPL 的 gs 继续只服务压缩,合规面不扩大。
3. **打包账**:M2 唯一的打包新工作是捆绑 libpdfium(自包含 dylib,较 gs 容易),
   且可以和 M3 的 gs sidecar 打包解耦——pdfium 捆绑先落地,PDF→图片立即可用,
   不被 gs 打包问题连坐。
4. **降级路径是安全的**:水印(pdfium 备选)、PDF→图片(gs 备选)都留了切换点,
   且切换方向都是"已经引进来的引擎",不会推翻重来。

## 5. 风险与开放问题

| # | 风险 | 影响 | 缓解 |
|---|------|------|------|
| R1 | **AGPL 履约(M3 打包 gs 后)**:需随发行物标注 Ghostscript 及 AGPL v3、提供对应源码获取方式、保留版权声明;Artifex 主张"与非 AGPL 应用同介质分发/应用含调用 gs 的功能"即需整体 AGPL 或商业许可([Artifex 说明](https://help-us.na.smart-integration.ricoh.com/PrintCloud%20Virtual%20Print%20Driver/Help/index6.html)),与"独立进程=聚合体,宿主可维持 MIT"的社区观点([讨论](https://opensource.stackexchange.com/questions/6879/may-i-use-agpl-license-in-a-desktop-application-without-providing-the-source-cod))存在分歧 | 挡闭源分发与 Mac App Store 上架;解释分歧是法律灰区 | 维持现状(不捆绑→义务未触发);M3 捆绑前做一次正式法务/合规复核(**待验证**);上架需求出现时买 Artifex 商业许可或彻底去 gs 化 |
| R2 | **pdfium dylib 捆绑与公证**:macOS 公证要求 bundle 内所有二进制签名;dylib 的加载路径、hardened runtime 下的 `@rpath` 需要处理;pdfium-binaries 每周一出新版,升级跟进是长期成本 | 打包失败或公证被拒 | M2 内先做捆绑+签名+公证的最小实验(spike);钉死 pdfium 版本,升级走显式变更;可选 `pdfium-bundled` 的 SHA-256 钉死方案做供应链校验 |
| R3 | **CJK 非嵌入字体度量(水印)**:STSong-Light 替换字体的实际字宽各家阅读器不一(先例实测行宽是估算值 1.12 倍,需乘 1.25 兜底),居中/长水印换行会偏 | 中文水印位置/大小不准 | 四阅读器(Adobe Reader/PDFKit/Chrome/MuPDF)实测;偏差不可接受时切 pdfium 嵌字方案(§3.1 触发条件) |
| R4 | **lopdf 保存兼容性**:追加内容流后整体重新序列化,对线性化、压缩对象流、加密件等边缘文件的行为需要真实语料验证;加密文档需先解密(沿用拆分/合并的既有取舍) | 个别文件输出损坏 | 用真实 PDF 语料 + 外部阅读器回归(沿用 M1 的 PDFKit 验证法);错误路径删半成品(沿用 compress.rs 模式) |
| R5 | **pdfium-render pre-1.0 API 变更**:0.9.x 迭代快,存在破坏性升级 | 升级成本 | 锁定次版本;渲染/水印调用收敛到薄封装层,升级只改一处 |
| R6 | **gs 转图备选的质量/速度**:色彩管理、透明度渲染瑕疵;逐进程启动慢 | 转换体验差 | 仅作降级路径;选它就明确接受(见 §3.3 触发条件) |
| R7 | **大图内存**:图片→PDF 与 PDF→图片都会遇到超高分辨率输入(扫描件 100MP 级) | 内存峰值、崩溃 | 流式/分页处理;限制单页渲染 DPI 上限并在 UI 标注 |

**开放问题**(需要产品侧拍板):

1. 水印一期是否只要**文字**,图片水印/二维码水印排后?(影响 §3.1 的实现顺序)
2. 中文水印字体策略:非嵌入(文件小,排版近似)vs 内嵌一份 Noto Sans SC 子集
   (约几 MB,排版精确)?与 R3 的实测结果联动。
3. 转换输出:PDF→图片要 PNG 还是 JPEG 还是可选?DPI 预设(网页 96/阅读 150/
   打印 300)?每页一图,是否要"长图拼接"?
4. 图片→PDF 页面策略:按图片原始 DPI 折算页尺寸,还是统一 A4 居中留白?
5. M3 跨平台时 PDF→图片是否保持 pdfium(建议是,这样 Windows/Linux 只是换
   动态库)?

## 6. 里程碑拆分建议

原则:M2a/M2b 无引擎依赖、可立即开工;需要打包的先做 spike,把不确定性前移。

### M2a —— 水印(lopdf,纯 Rust,可立即开工)

- [ ] `watermark_pdf` 命令:文字水印(位置/旋转/透明度/字号)叠加到每页
  - [ ] 内容流追加:`/Contents` 间接数组的坑(见 §2.4 先例)、`q…Q` 包裹防止
        原 CTM 泄漏
  - [ ] Latin(基 14 Helvetica)+ CJK(Adobe-GB1 非嵌入)双路;`/ToUnicode` 保证
        可复制
  - [ ] CropBox/MediaBox 取页面真实边界(带出血文件不从 (0,0) 起算)
- [ ] 图片水印:Image XObject + 缩放/透明度
- [ ] 四阅读器回归(R3):Adobe Reader / PDFKit / Chrome / MuPDF 验证中文排版
- [ ] 单元/集成测试沿用现有模式(`gs_args` 式纯函数拆分:参数构建与执行分离)

### M2b —— 转换(图片→PDF 先行)

- [ ] `images_to_pdf`(printpdf):多图→多页、DPI/页面策略、JPEG/PNG/WebP 解码
- [ ] `pdf_to_images` 入口与 UI 骨架先行,渲染实现依赖下面的捆绑 spike

### M2c —— pdfium 捆绑 spike(把 M3 前置不确定性前移)

- [ ] 下载钉死版本的 pdfium-binaries mac-arm64/x64 动态库,进 app bundle
  (Tauri `bundle.resources` 或等价机制)
- [ ] `pdfium-render` 运行时绑定:同目录 `bind_to_library` → 回退系统库(照抄
  crate examples 的惯用法)
- [ ] 代码签名 + 公证走通(R2)
- [ ] `has_render_engine` 命令 + 页面友好提示(沿用 `has_compression_engine` 模式)
- [ ] 体积与首启耗时实测,回填 §2.1 的"待实测"

### M2d —— PDF→图片(pdfium)

- [ ] 逐页栅格化:DPI/尺寸/透明背景选项,PNG/JPEG 输出
- [ ] 进度事件(长任务,配合 M3 的进度 UI 约定提前设计事件名)
- [ ] 大图/多页内存上限(R7)

### M3(与 ROADMAP 对齐的增补)

- [ ] **gs 打包(M3 前置)**:静态化或 `install_name_tool` 重写 + `bundle.externalBin`;
      履行 AGPL 义务(R1,发布前法务复核)
- [ ] 打包策略定稿:pdfium(M2c)与 gs(M3)并存时的体积预算、是否用 pdfium
      长期替换 gs 的部分功能(压缩仍需 gs)
- [ ] 跨平台:Windows/Linux 的 pdfium 动态库按 target triple 分发
      (`gs-<target-triple>` 命名约定同款)
- [ ] 其余原 ROADMAP 项(CSP、进度 UI、E2E、签名公证)不变

## 附:来源清单

| 论断 | 来源 |
|------|------|
| pdfium-render 版本/活跃度/API(watermark、字体加载) | [crates.io](https://crates.io/crates/pdfium-render)、[docs.rs](https://docs.rs/pdfium-render/latest/pdfium_render/)、[GitHub](https://github.com/ajrcarey/pdfium-render)(含 [examples/watermark.rs](https://github.com/ajrcarey/pdfium-render/blob/master/examples/README.md)) |
| PDFium 许可 BSD-3-Clause + Apache-2.0 | [pdfium 上游 LICENSE](https://pdfium.googlesource.com/pdfium/+/main/LICENSE) |
| pdfium-binaries:MIT、每周构建、仅动态库 | [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/) |
| pdfium 捆绑/钉死校验方案与体积参考 | [pdfium-bundled](https://docs.rs/pdfium-bundled/latest/pdfium_bundled/) |
| 第三方应用的 pdfium 履约声明样式 | [Filoshi 开源声明](https://filoshi.com/licenses/pdfium.txt) |
| mupdf-rs AGPL、从源码构建 MuPDF | [messense/mupdf-rs](https://github.com/messense/mupdf-rs)、[crates.io/mupdf](https://crates.io/crates/mupdf)、[fileformats.dev](https://fileformats.dev/xps/libraries/rust/mupdf) |
| Artifex AGPL/商业双许可条款 | [Artifex licensing](https://artifex.com/licensing)、[Artifex 分发说明](https://help-us.na.smart-integration.ricoh.com/PrintCloud%20Virtual%20Print%20Driver/Help/index6.html) |
| AGPL"库集成即整体"与桌面聚合观点分歧 | [OpenSource.SE: Bruno Lowagie 回答](https://opensource.stackexchange.com/questions/6879/may-i-use-agpl-license-in-a-desktop-application-without-providing-the-source-cod)、[AGPL §13 解读](https://opensource.com/article/17/1/providing-corresponding-source-agplv3-license) |
| gs 水印做法(EndPage/pdfmark)与重写文档副作用 | [StackOverflow/KenS](https://stackoverflow.com/questions/38693217/how-add-watermark-to-ghostscript-with-command-line)、[Transloadit devtip](https://transloadit.com/devtips/automate-pdf-watermarking-with-ghostscript/)、[rsmith overlay howto](https://rsmith.home.xs4all.nl/howto/adding-an-overlay-to-each-page-of-a-pdf-file.html) |
| lopdf 追加内容流 API | [docs.rs/lopdf](https://docs.rs/lopdf/latest/lopdf/struct.Document.html)(`add_page_contents`) |
| lopdf 水印实现先例与实测坑 | [cap-doc watermark.rs](https://docs.rs/cap-doc/latest/src/cap_doc/pdf/watermark.rs.html)、[pdf-manip](https://docs.rs/pdf-manip/latest/src/pdf_manip/watermark.rs.html)、[pdforg](https://docs.rs/pdforg/latest/src/pdforg/pdf_tools/stamp.rs.html) |
| printpdf 能力(图片→PDF、page_to_svg 实验)与 MIT | [crates.io/printpdf](https://crates.io/crates/printpdf)、[docs.rs/printpdf](https://docs.rs/printpdf/latest/printpdf/)、[fschutt/printpdf](https://github.com/fschutt/printpdf) |
| krilla 能力/许可/typst 后端 | [crates.io/krilla](https://crates.io/crates/krilla)、[LaurenzV/krilla](https://github.com/LaurenzV/krilla)、[typst#5420](https://github.com/typst/typst/pull/5420) |
| qpdf Apache-2.0、poppler GPL-2.0+、veraPDF GPL-3.0+/MPL-2.0 | [qpdf license](https://qpdf.readthedocs.io/en/stable/license.html)、[veraPDF](https://verapdf.org/home/)、[PDF 库许可汇总表(2026-09)](https://wholefiles.com/guides/pdf-library-licences/)、[开源 PDF 工具许可记录](https://dialegein.com/guides/open-source/) |

**待验证项汇总**:libpdfium/sidecar 实际体积与首启耗时(R2/§2.1);CJK 替换字体
行宽偏差的多阅读器实测(R3);lopdf 对真实语料的保存兼容性(R4);Artifex 分发
主张的法律效力(R1,需法务);typst 许可文本复核(§2.4);PDFKit 作临时渲染后端的
Rust 集成成本(§3.3)。
