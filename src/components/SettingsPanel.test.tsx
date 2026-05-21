import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test, vi } from "vitest";
import "../locales/test-setup";
import { SettingsPanel } from "./SettingsPanel";

const config = {
  locale: "zh-CN",
  notesDir: "D:\\Notes\\花笺",
  globalShortcut: "Ctrl+Space",
  closeToTray: true,
  autostart: false,
  defaultViewMode: "split" as const,
  noteAutoSave: true,
  noteSurfaceAutoSave: true,
  tileColor: "#f6f3ec",
  tileColorMode: "custom" as const,
  theme: "light" as const,
  fontSize: 14,
  surfaceFontSize: 14,
  externalFileAutoSave: true,
  rememberSurfaceSize: true,
  tileCtrlClose: true,
  toggleVisibilityShortcut: "",
  tileRenderMarkdown: false,
  syncEnabled: true,
  syncServerUrl: "https://notes.example.com",
  syncToken: "secret-token",
};

describe("SettingsPanel", () => {
  test("renders the core configurable app settings", () => {
    const markup = renderToStaticMarkup(
      <SettingsPanel
        config={config}
        onChange={vi.fn()}
        onChooseNotesDir={vi.fn()}
        onClose={vi.fn()}
        syncStatus={{
          enabled: true,
          configured: true,
          lastRevision: "9",
          lastSyncAt: "2026-05-18T08:00:00Z",
          lastError: null,
        }}
        onSyncNow={vi.fn()}
        onTestSyncConnection={vi.fn()}
      />,
    );

    expect(markup).toContain("应用设置");
    expect(markup).toContain("D:\\Notes\\花笺");
    expect(markup).toContain("选择文件夹");
    expect(markup).toContain("Ctrl+Space");
    expect(markup).toContain("关闭到托盘");
    expect(markup).toContain("开机自启");
    expect(markup).toContain("自动保存笔记");
    expect(markup).toContain("小窗笔记自动保存");
    expect(markup).toContain("磁贴颜色");
    expect(markup).toContain("跟随主题");
    expect(markup).toContain("自定义");
    expect(markup).toContain('type="color"');
    expect(markup).toContain('value="#f6f3ec"');
    expect(markup).toContain("同步");
    expect(markup).toContain("https://notes.example.com");
    expect(markup).toContain("立即同步");
    expect(markup).toContain("测试连接");
    expect(markup).toContain("默认视图");
    expect(markup).toContain("编辑");
    expect(markup).toContain("分栏");
    expect(markup).toContain("预览");
  });

  test("renders friendly sync feedback and errors", () => {
    const successMarkup = renderToStaticMarkup(
      <SettingsPanel
        config={config}
        onChange={vi.fn()}
        onChooseNotesDir={vi.fn()}
        onClose={vi.fn()}
        syncStatus={{
          enabled: true,
          configured: true,
          lastRevision: "9",
          lastSyncAt: null,
          lastError: null,
        }}
        syncFeedback={{ tone: "success", message: "连接成功，可以开始同步。" }}
        onSyncNow={vi.fn()}
        onTestSyncConnection={vi.fn()}
      />,
    );

    const errorMarkup = renderToStaticMarkup(
      <SettingsPanel
        config={config}
        onChange={vi.fn()}
        onChooseNotesDir={vi.fn()}
        onClose={vi.fn()}
        syncStatus={{
          enabled: true,
          configured: true,
          lastRevision: "9",
          lastSyncAt: null,
          lastError: "无法连接到同步服务器，请检查地址、端口和网络。",
        }}
        onSyncNow={vi.fn()}
        onTestSyncConnection={vi.fn()}
      />,
    );

    expect(successMarkup).toContain("连接成功，可以开始同步。");
    expect(errorMarkup).toContain("无法连接到同步服务器，请检查地址、端口和网络。");
  });
});
