# phi-tui

[![Crates.io](https://img.shields.io/crates/v/phi-tui.svg)](https://crates.io/crates/phi-tui)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

面向 LLM 终端的聊天式 TUI 组件集,基于 ratatui + crossterm。
"人跟 LLM 打字聊天"的终端产品所需要的全部部件:带可锚定替换块的转录缓冲、
流式尾部状态机、CJK 感知的折行、滚动视口、鼠标选区复制、markdown 渲染,
以及 `@`/`/` 补全 picker。

**零 agent 语义。** phi-tui 不知道审批、工具调用、子 agent 是什么——
那些属于产品层。除 `ratatui` / `crossterm` / `pulldown-cmark` /
`unicode-width` / `tracing` 外无任何框架依赖。

## 组件一览

| 模块 | 能力 |
|---|---|
| `lines` | 行模型:`OutputLine<S>`、`LineKind`、`ToolDetail`、`DiffHunk`,通用样式字节区间 `SpanSpec<S>` |
| `transcript` | 已提交转录缓冲 + 锚定替换块 + 宽度变化重排 |
| `stream` | 流式尾部状态机:增量进、整行出,渲染 O(新增字节) |
| `wrap` | CJK 感知硬折行(`wrap` / `one_line`)+ 增量 `WrapCache` |
| `viewport` | 滚动状态:偏移、贴底跟随、可见窗口区间 |
| `selection` | 鼠标选区 + 右键复制菜单(纯文本进、纯文本出) |
| `markdown` | pulldown-cmark → `Vec<ratatui::Line>`:无标记标题、分隔线、行内样式、表格、LaTeX → Unicode |
| `picker` / `completer` / `mention` | 共享补全状态机 + `@` 路径补全(纯 fs,不依赖终端) |
| `input` | 多行输入 `Composer`(Shift+Enter 换行,粘贴安全) |
| `diff` | 手写 LCS 行级 diff → unified hunk |

行模型对产品的样式 token 泛型:`OutputLine<S = ()>`——纯文本场景直接用
`OutputLine<()>`,或钉上自己的 token(颜色、语义角色),以
`spans: Option<Vec<SpanSpec<S>>>` 字节区间挂到纯文本上,
复制/选区与帧捕获始终对样式无感。

## 用法

```toml
[dependencies]
phi-tui = "0.1"
```

```rust
use phi_tui::transcript::Transcript;
use phi_tui::viewport::Viewport;

let mut t: Transcript = Transcript::new();
t.push_user("hello");
let mut vp = Viewport::new();
let window = vp.window_range(t.len(), 10); // 本帧要画的行区间
```

把流式增量提交成整行:

```rust
use phi_tui::stream::StreamState;
use phi_tui::transcript::{Transcript, DEFAULT_WRAP_WIDTH};

let mut t: Transcript = Transcript::new();
let mut st: StreamState = StreamState::new(DEFAULT_WRAP_WIDTH);
st.push_text("hel", None);
st.push_text("lo", None);
let lines = st.flush(); // → 一条已提交的 `OutputLine` "hello"
t.extend(lines);
```

完整示例:`examples/chat-demo.rs`(约 200 行的端到端迷你聊天 TUI,
`cargo run --example chat-demo`)与 `examples/picker-demo.rs`(补全状态机)。

## 设计要点

- **纯文本对样式无感**:`OutputLine.text` 永远是纯文本拼接;样式以
  `SpanSpec { start, len, style }` 字节区间携带,复制/选区与帧捕获
  永远看不到样式。
- **锚定替换块**:`Transcript::replace_plan` / `clear_plan` 实现
  "整块替换、原位更新"——plan/todo 类实时刷新组件背后的通用模式。
- **状态机 + 薄渲染**:每个组件都是纯状态机或纯函数;渲染发生在
  你自己的帧循环里。

## 许可

MIT —— 见 [LICENSE](LICENSE)。
