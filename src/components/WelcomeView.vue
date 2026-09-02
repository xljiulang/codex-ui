<script setup lang="ts">
import { onMounted, ref } from "vue";
import { getVersion } from "@tauri-apps/api/app";

/** 当前应用版本（非 Tauri 环境/读取失败时隐藏，不影响页面展示） */
const appVersion = ref("");

onMounted(async () => {
  try {
    appVersion.value = await getVersion();
  } catch {
    // 非 Tauri 环境（浏览器预览/单测）静默忽略
  }
});
</script>

<template>
  <div class="welcome-view">
    <div class="welcome-inner">
      <header class="welcome-hero">
        <h1 class="welcome-title">欢迎使用 Codex-UI</h1>
        <p class="welcome-tagline">
          基于 Rust + Tauri 2 的 Codex CLI Windows 桌面客户端：以本地桌面会话驱动
          Codex，会话、文件、Git 与终端可在多标签中并行工作。
        </p>
        <p class="welcome-guide">
          点标签栏末尾「＋」新建会话或终端；或在右侧历史会话面板打开、绑定既有会话后继续对话。
        </p>
        <span v-if="appVersion" class="welcome-version">v{{ appVersion }}</span>
      </header>

      <section aria-label="核心功能">
        <h2 class="welcome-section-title">核心功能</h2>
        <div class="feature-grid">
          <article class="feature-card">
            <h3>会话管理 · 微信远控 <span class="feature-badge warn">实验性</span></h3>
            <p>
              历史会话上右键「微信接入」，扫码即可把 Codex 带进微信聊天：人在外面发消息，
              Codex 在家里干活，回复以绑定账号被动发回。
            </p>
            <ul class="feature-list">
              <li>本人门禁：只处理扫码绑定账号本人发来的消息，其他消息一律忽略</li>
              <li>微信触发回合固定为完全访问 + 免审批；回复保留 Markdown 原文、超长自动分段</li>
              <li>支持多个会话各自绑定不同微信账号；删除会话自动解除绑定</li>
            </ul>
          </article>

          <article class="feature-card">
            <h3>
              对话内动态工具（codexui）
              <span class="feature-badge warn">实验性</span>
            </h3>
            <p>
              Codex-UI 新建会话时注入 <code>codexui</code> 命名空间动态工具，Codex
              可在回合内自行调用；工具调用由 codex-ui 直接应答，不打断回合。
            </p>
            <ul class="feature-list">
              <li>
                <code>codexui_get_usage</code>：查询当前会话 token 消耗与上下文窗口占用
              </li>
              <li>
                <code>codexui_compact_context</code>：接近上限时主动请求压缩上下文
              </li>
              <li>
                微信远控的必要：绑定这类会话后你不在电脑旁、够不到手动压缩按钮，Codex
                可自查用量并主动压缩，避免长任务/多轮对话因上下文耗尽中断或质量下降
              </li>
            </ul>
            <p class="feature-note">
              动态工具依赖 experimentalApi 且仅随 Codex-UI 新建会话注入（历史/分叉线程无）；
              用 Codex-UI 新建会话绑定微信后，远控回合即可自维护。
            </p>
          </article>

          <article class="feature-card">
            <h3>资源管理</h3>
            <p>
              右侧「资源」面板以当前标签的工作区为根展示文件树：懒加载、搜索、自动刷新、
              右键管理、拖拽移动，文件变化实时同步。
            </p>
            <ul class="feature-list">
              <li>文本与代码：内置编辑器，语法高亮、可编辑、一键代码格式化</li>
              <li>Markdown 预览/编辑切换、.docx 富文本编辑</li>
              <li>.pdf、常见图片（png/jpg/gif/webp/bmp/svg/ico/avif 等）只读预览</li>
              <li>.xlsx 表格预览、Git diff 标签对比</li>
            </ul>
          </article>

          <article class="feature-card">
            <h3>Git</h3>
            <p>
              全部调用系统 git.exe：以活动标签工作区检测仓库，变更自动刷新并列出
              已更改/已暂存文件，点击即开 diff 标签。
            </p>
            <ul class="feature-list">
              <li>分支管理：切换/新建/删除/合并，含远程分支与远端管理</li>
              <li>提交历史逐批加载；拉取快进优先，推送自动设置上游</li>
              <li>合并冲突自动中止回滚；非仓库目录可一键 git init</li>
            </ul>
          </article>

          <article class="feature-card">
            <h3>日志防膨胀</h3>
            <p>
              自动为 codex 的日志库应用写阻断触发器，从源头丢弃持续写入的 TRACE 日志，
              防止 logs_2.sqlite 及其 WAL 无限增长。
            </p>
            <ul class="feature-list">
              <li>连接握手成功后自动定位 CODEX_HOME 下的日志库并应用，幂等无感</li>
              <li>失败仅记录日志，不影响 codex 正常工作</li>
            </ul>
          </article>

          <article class="feature-card">
            <h3>内置双插件市场</h3>
            <p>
              启动即自动注册两个插件市场，设置页「插件管理」中按市场分组安装/卸载插件。
            </p>
            <ul class="feature-list">
              <li>openai-bundled：随安装包提供、离线可用</li>
              <li>openai-primary-runtime：按需从 CDN 下载并解压，支持断点续传</li>
              <li>支持添加/移除自定义市场，插件接口图标自动展示</li>
            </ul>
          </article>

          <article class="feature-card">
            <h3>多会话并行标签</h3>
            <p>
              左侧标签区可同时打开多个会话、文件、预览与终端标签，切换互不中断。
            </p>
            <ul class="feature-list">
              <li>后台会话继续运行，回合完成/待交互以角标提示</li>
              <li>每个会话独立权限模式、模型、任务模式与目标</li>
              <li>标签常驻挂载：切走不销毁编辑状态与进程</li>
            </ul>
          </article>

          <article class="feature-card">
            <h3>终端</h3>
            <p>
              内置 ConPTY 终端（PowerShell / cmd），随资源文件夹或历史会话分组一键启动。
            </p>
            <ul class="feature-list">
              <li>支持同目录多开，切走标签不中断命令执行</li>
              <li>命令执行中标签显示呼吸圆点，回到提示符自动熄灭</li>
              <li>标签标题可右键重命名；配色随三套主题即时切换</li>
            </ul>
          </article>
        </div>
      </section>

      <section aria-label="更多亮点">
        <h2 class="welcome-section-title">更多亮点</h2>
        <div class="highlight-grid">
          <div class="highlight-item">历史会话按目录分组、搜索、置顶与分叉</div>
          <div class="highlight-item">首条消息自动总结标题，随时查看 token 用量</div>
          <div class="highlight-item">上下文窗口占用提示与一键压缩</div>
          <div class="highlight-item">只读/请求批准/帮我批准/完全访问四种权限</div>
          <div class="highlight-item">执行/计划任务模式与目标旗子自动续跑</div>
          <div class="highlight-item">蓝夜/曜黑/晨光三套主题</div>
          <div class="highlight-item">技能、MCP 与插件集中管理</div>
          <div class="highlight-item">本地记忆开关与记忆管理</div>
          <div class="highlight-item">系统托盘常驻：关窗不退出、微信与 Codex 保持在线</div>
          <div class="highlight-item">为 agent 注入 rg / fd / ast-grep 工具说明</div>
        </div>
      </section>

      <footer class="welcome-foot">
        Codex-UI · 仅适配 codex-cli 0.149.x（验证基线 0.149.0）<br />
        更完整的功能说明见项目 README 与 docs/变更记录.md
      </footer>
    </div>
  </div>
</template>
