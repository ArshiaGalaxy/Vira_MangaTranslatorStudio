"""Manga/comic translation engine.

Importing this package preloads torch when present (Windows DLL-order fix,
see below). torch is optional since LaMa inpainting moved to ONNX Runtime.
"""

__version__ = "2.0.1"

# ── Windows DLL-order fix (PaddleOCR vs torch, only when torch is installed) ─
# PaddleOCR (paddlepaddle) and torch both ship ``libiomp5md.dll``. If paddle is
# imported first (during the Extract/OCR step), torch later fails to load with
# ``OSError: [WinError 127] ... torch\lib\shm.dll`` because the conflicting
# OpenMP DLL is already in the process. Pre-importing torch here — before any
# PaddleOCR code runs — lets torch's OpenMP DLL win the load order. With the
# ONNX LaMa swap torch is no longer a dependency; this stays as a no-op
# safety net for environments that still have it installed.
try:
    import torch  # noqa: F401  (preload to resolve the DLL conflict)
except Exception:
    pass
