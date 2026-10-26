#!/usr/bin/env python3
"""Render src-tauri/icons/*.png from bambuddy-monitor.svg (needs PySide6)."""
import os
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
from PySide6.QtCore import Qt
from PySide6.QtGui import QGuiApplication, QImage, QPainter
from PySide6.QtSvg import QSvgRenderer

app = QGuiApplication([])
r = QSvgRenderer("bambuddy-monitor.svg")
for size, name in ((32, "32x32"), (128, "128x128"), (256, "128x128@2x"), (512, "icon")):
    img = QImage(size, size, QImage.Format_ARGB32)
    img.fill(Qt.transparent)
    p = QPainter(img)
    r.render(p)
    p.end()
    img.save(f"src-tauri/icons/{name}.png")
