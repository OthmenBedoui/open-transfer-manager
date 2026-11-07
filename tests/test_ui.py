"""Opens the window and every dialog in both themes (skipped when there is no display)."""

import unittest

import helpers  # noqa: F401

try:
    import tkinter as tk
    _root = tk.Tk()
    _root.destroy()
    HAVE_DISPLAY = True
except Exception:  # noqa: BLE001
    HAVE_DISPLAY = False


@unittest.skipUnless(HAVE_DISPLAY, "no display")
class UiTest(unittest.TestCase):
    def test_window_and_dialogs(self):
        from otm.core import engine
        from otm.ui import app as appmod, dialogs
        root = tk.Tk()
        try:
            mgr = engine.Manager(persist=False)
            mgr.downloads = appmod.demo_downloads(mgr)
            app = appmod.App(root, mgr, "light")
            root.update()
            for mode in ("dark", "light"):
                app.theme.set_mode(mode)
                root.update()
            for view in ("active", "unfinished", "finished", "failed", "cat/Video", "all"):
                app.select_view(view)
                root.update()
            app.list.selection_set(app.list.get_children()[0])
            app._update_details()
            app._sort("size")
            root.update()
            for make in (lambda: dialogs.AddDialog(app, "https://example.com/file.zip"),
                         lambda: dialogs.SettingsDialog(app),
                         lambda: dialogs.SettingsDialog(app, tab="media"),
                         lambda: dialogs.AboutDialog(app),
                         lambda: dialogs.DiagnosticsDialog(app),
                         lambda: dialogs.DeleteDialog(app, mgr.downloads[:2])):
                w = make()
                root.update()
                try:
                    w.grab_release()
                except tk.TclError:
                    pass
                w.destroy()
            self.assertTrue(app.list.get_children())
        finally:
            root.destroy()


if __name__ == "__main__":
    unittest.main()
