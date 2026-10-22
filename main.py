"""
Local background-removal studio (BiRefNet).

Pages:
    /            Node compositor (Svelte Flow UI built into frontend/dist)
    /extract     Classic single-image background remover
    /docs        FastAPI API docs

API:
    POST /api/remove   remove the background, returns RGBA PNG
    POST /api/mask     soft alpha mask as white RGB + alpha PNG
    POST /api/restore  GFPGAN face restoration, returns PNG
    GET  /api/fetch    server-side image download for URL previews
    GET  /api/files    images in the local ./input and ./output folders
    GET  /api/workspaces/{templates|projects}
                        list saved graphs (shared output images included)
    GET  /input, /output  read-only file access for those folders
    GET  /workspaces   legacy saved template/project files
    GET  /health       model/device status

Storage:
    Masks and restores are cached under ./output/cache and keyed by image
    content plus settings (model, strength, faces). Repeating an operation on
    the same pixels returns the stored PNG without loading the model. Every
    new result is also copied to ./output as <source>.<rembg|restored>.<key6>.png.
    Source images, masks, restores, and thumbnails used by saved templates and
    projects live in ./output/shared, so identical data is reused by every
    workspace instead of being copied into each one.

Setup (once):
    pip install torch torchvision --index-url https://download.pytorch.org/whl/cu118
    pip install -r requirements.txt
    cd frontend && npm install && npm run build && cd ..

Run:
    python main.py
Then open http://127.0.0.1:8000

Both models are loaded lazily on their first request, so startup is instant.
They are freed when the app stops, via POST /api/unload, or after
MODEL_IDLE_TIMEOUT seconds without requests (0 = keep them loaded).
A local copy in ./models/BiRefNet is used automatically when present (offline friendly).
GFPGAN is loaded from ./models/GFPGANv1.4.pth with facexlib weights in
./models/facexlib (detection_Resnet50_Final.pth + parsing_parsenet.pth).
If frontend/dist is missing, / falls back to the single-image remover.
Optional env vars:
    MODEL_ID=ZhengPeng7/BiRefNet_lite   (lighter model if you run out of VRAM)
    MODEL_IDLE_TIMEOUT=300              (unload after N idle seconds; 0 = never)
    FACE_MODEL_PATH=...                 (override ./models/GFPGANv1.4.pth)
    CACHE_DIR=...                       (override ./output/cache)
    INPUT_DIR=...                       (override ./input)
    OUTPUT_DIR=...                      (override ./output)
    WORKSPACES_DIR=...                  (override ./workspaces)
    PORT=8000
"""

import gc
import hashlib
import io
import json
import mimetypes
import os
import re
import shutil
import threading
import time
import uuid
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from typing import Optional
from urllib.parse import urlparse

import numpy as np
import requests
import uvicorn
from fastapi import Body, FastAPI, File, Form, HTTPException, Request, UploadFile
from fastapi.responses import FileResponse, HTMLResponse, Response
from fastapi.staticfiles import StaticFiles
from PIL import Image, ImageOps

BASE_DIR = os.path.dirname(os.path.abspath(__file__))
LOCAL_MODEL_DIR = os.path.join(BASE_DIR, "models", "BiRefNet")
MODEL_ID = os.environ.get("MODEL_ID") or (
    LOCAL_MODEL_DIR if os.path.isdir(LOCAL_MODEL_DIR) else "ZhengPeng7/BiRefNet"
)
PORT = int(os.environ.get("PORT", 8000))
INPUT_SIZE = (1024, 1024)
MAX_DOWNLOAD_BYTES = 30 * 1024 * 1024
MAX_PREVIEW_SIDE = 4096
MODEL_IDLE_TIMEOUT = float(os.environ.get("MODEL_IDLE_TIMEOUT") or 0)
FACE_MODEL_PATH = os.environ.get("FACE_MODEL_PATH") or os.path.join(
    BASE_DIR, "models", "GFPGANv1.4.pth"
)
FACE_AUX_DIR = os.path.join(BASE_DIR, "models", "facexlib")

# Results are cached on disk by image content + settings, so the same pixels
# are never sent through the models twice. `input/` is a drop folder for the
# user; every processed result is also copied to `output/` with a readable name.
# Both the model cache and the images referenced by saved workspaces live in
# `output/`, so every template and project shares one content-addressed pool.
INPUT_DIR = os.environ.get("INPUT_DIR") or os.path.join(BASE_DIR, "input")
OUTPUT_DIR = os.environ.get("OUTPUT_DIR") or os.path.join(BASE_DIR, "output")
CACHE_DIR = os.environ.get("CACHE_DIR") or os.path.join(OUTPUT_DIR, "cache")
SHARED_DIR = os.path.join(OUTPUT_DIR, "shared")
SHARED_ASSET_DIR = os.path.join(SHARED_DIR, "assets")
SHARED_MASK_DIR = os.path.join(SHARED_DIR, "masks")
SHARED_RESTORE_DIR = os.path.join(SHARED_DIR, "restores")
SHARED_THUMB_DIR = os.path.join(SHARED_DIR, "thumbs")
WORKSPACES_DIR = os.environ.get("WORKSPACES_DIR") or os.path.join(
    BASE_DIR, "workspaces"
)
CACHE_VERSION = "1"  # bump to invalidate every cached result
IMAGE_SUFFIXES = (".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif", ".tif", ".tiff")
WORKSPACE_KINDS = {"templates", "projects"}

os.makedirs(INPUT_DIR, exist_ok=True)
os.makedirs(OUTPUT_DIR, exist_ok=True)
for _sub in ("mask", "restore"):
    os.makedirs(os.path.join(CACHE_DIR, _sub), exist_ok=True)
for _shared in (
    SHARED_ASSET_DIR,
    SHARED_MASK_DIR,
    SHARED_RESTORE_DIR,
    SHARED_THUMB_DIR,
):
    os.makedirs(_shared, exist_ok=True)
for _kind in WORKSPACE_KINDS:
    os.makedirs(os.path.join(WORKSPACES_DIR, _kind), exist_ok=True)


def _migrate_legacy_cache() -> None:
    """Move legacy ./cache mask/restore PNGs into ./output/cache once."""
    legacy = os.path.join(BASE_DIR, "cache")
    target = os.path.join(OUTPUT_DIR, "cache")
    if os.path.abspath(CACHE_DIR) != os.path.abspath(target):
        return
    if not os.path.isdir(legacy) or os.path.abspath(legacy) == os.path.abspath(target):
        return
    for sub in ("mask", "restore"):
        src_dir = os.path.join(legacy, sub)
        if not os.path.isdir(src_dir):
            continue
        dst_dir = os.path.join(CACHE_DIR, sub)
        os.makedirs(dst_dir, exist_ok=True)
        for name in os.listdir(src_dir):
            if not name.endswith(".png"):
                continue
            src = os.path.join(src_dir, name)
            dst = os.path.join(dst_dir, name)
            try:
                if os.path.exists(dst):
                    os.remove(src)
                else:
                    shutil.move(src, dst)
            except OSError:
                pass
        try:
            os.rmdir(src_dir)
        except OSError:
            pass
    try:
        os.rmdir(legacy)
    except OSError:
        pass


_migrate_legacy_cache()

# BiRefNet and GFPGAN share one GPU: serialize forwards so they never overlap.
INFERENCE_LOCK = threading.Lock()


class ModelManager:
    """
    Lazily load BiRefNet on first use and free it on demand, when idle, or at shutdown.

    The heavy imports (torch, torchvision, transformers) live inside _load so
    importing main.py - and starting the server - stays fast.
    """

    def __init__(self, model_id: str, idle_timeout: float = 0.0):
        self.model_id = model_id
        self.idle_timeout = idle_timeout
        self._model = None
        self._device: Optional[str] = None
        self._tfm = None
        self._torch = None
        self._cond = threading.Condition()  # guards load/unload + busy counter
        self._active = 0  # requests currently using the model
        self._idle_timer: Optional[threading.Timer] = None
        self._loading = False
        self._closed = False

    @property
    def loaded(self) -> bool:
        return self._model is not None

    @property
    def device(self) -> Optional[str]:
        return self._device

    @property
    def status(self) -> str:
        if self._model is not None:
            return "loaded"
        return "loading" if self._loading else "unloaded"

    def acquire(self) -> tuple[str, object]:
        """Load the model if needed and keep it busy until release()."""
        with self._cond:
            if self._closed:
                raise RuntimeError("Model manager is shut down")
            self._cancel_idle_timer()
            if self._model is None:
                self._load()
            self._active += 1
            return self._device, self._tfm

    def release(self) -> None:
        with self._cond:
            self._active = max(0, self._active - 1)
            if self._active == 0:
                self._cond.notify_all()
                self._arm_idle_timer()

    def predict(self, x):
        """Run one forward pass (serialized) and return the mask tensor."""
        with INFERENCE_LOCK, self._torch.inference_mode():
            return self._model(x)[-1].sigmoid().float().cpu()[0].squeeze()

    def unload(self) -> bool:
        """Free the model now; returns False if it is not loaded or still busy."""
        with self._cond:
            self._cancel_idle_timer()
            if self._model is None or self._active:
                return False
            self._unload()
            return True

    def close(self) -> None:
        """Shutdown hook: stop new loads, wait for in-flight work, free memory."""
        with self._cond:
            self._closed = True
            self._cancel_idle_timer()
            while self._active:
                self._cond.wait()
            self._unload()

    def _load(self) -> None:
        self._loading = True
        try:
            import torch
            from torchvision import transforms
            from transformers import AutoModelForImageSegmentation

            device = "cuda" if torch.cuda.is_available() else "cpu"
            print(f"Loading {self.model_id} on {device} ... (first run downloads the weights)")
            torch.set_float32_matmul_precision("high")
            model = AutoModelForImageSegmentation.from_pretrained(
                self.model_id,
                trust_remote_code=True,
                low_cpu_mem_usage=False,
                torch_dtype=torch.float16,
            )
            # The checkpoint ships fp16, so instantiating in fp16 and casting on
            # the device avoids building a second fp32 copy on the CPU:
            # ~1.3 GB peak host RAM instead of ~2.0 GB, and ~0.9 GB steady
            # instead of ~1.7 GB. Compute stays fp32 on purpose: GTX 16-series
            # cards give broken output with fp16 inference.
            model = model.to(device=device, dtype=torch.float32).eval()
            self._tfm = transforms.Compose(
                [
                    transforms.Resize(INPUT_SIZE),
                    transforms.ToTensor(),
                    transforms.Normalize([0.485, 0.456, 0.406], [0.229, 0.224, 0.225]),
                ]
            )
            self._torch = torch
            self._device = device
            self._model = model
            print("Model ready.")
        finally:
            self._loading = False

    def _unload(self) -> bool:
        if self._model is None:
            return False
        torch, device = self._torch, self._device
        self._model = self._device = self._tfm = self._torch = None
        gc.collect()
        if torch is not None and device == "cuda":
            torch.cuda.empty_cache()
        print("Model unloaded.")
        return True

    def _cancel_idle_timer(self) -> None:
        if self._idle_timer is not None:
            self._idle_timer.cancel()
            self._idle_timer = None

    def _arm_idle_timer(self) -> None:
        if self.idle_timeout <= 0 or self._model is None:
            return
        self._cancel_idle_timer()
        timer = threading.Timer(self.idle_timeout, self._idle_unload)
        timer.daemon = True
        timer.start()
        self._idle_timer = timer

    def _idle_unload(self) -> None:
        with self._cond:
            self._idle_timer = None
            if self._active == 0 and self._model is not None:
                self._unload()


MODEL = ModelManager(MODEL_ID, MODEL_IDLE_TIMEOUT)


def _tensor2img(tensor, rgb2bgr=True, min_max=(0.0, 1.0)) -> np.ndarray:
    """CHW torch tensor -> HWC uint8 numpy, without pulling in basicsr."""
    t = tensor.detach().squeeze(0).float().cpu().clamp(*min_max)
    t = (t - min_max[0]) / (min_max[1] - min_max[0])
    arr = (t.permute(1, 2, 0).numpy() * 255.0).round().astype(np.uint8)
    if rgb2bgr:
        arr = arr[:, :, ::-1]
    return np.ascontiguousarray(arr)


class FaceRestoreManager:
    """Lazily load GFPGAN + facexlib and free them on demand/idle/shutdown."""

    def __init__(self, model_path: str, aux_dir: str, idle_timeout: float = 0.0):
        self.model_path = model_path
        self.aux_dir = aux_dir
        self.idle_timeout = idle_timeout
        self._model = None
        self._helper = None
        self._device: Optional[str] = None
        self._torch = None
        self._cv2 = None
        self._img2tensor = None
        self._normalize = None
        self._cond = threading.Condition()
        self._active = 0
        self._idle_timer: Optional[threading.Timer] = None
        self._loading = False
        self._closed = False

    @property
    def loaded(self) -> bool:
        return self._model is not None

    @property
    def device(self) -> Optional[str]:
        return self._device

    @property
    def status(self) -> str:
        if self._model is not None:
            return "loaded"
        return "loading" if self._loading else "unloaded"

    @property
    def available(self) -> bool:
        return os.path.isfile(self.model_path)

    def acquire(self) -> tuple[str, object, object]:
        with self._cond:
            if self._closed:
                raise RuntimeError("Face restore manager is shut down")
            self._cancel_idle_timer()
            if self._model is None:
                self._load()
            self._active += 1
            return self._device, self._model, self._helper

    def release(self) -> None:
        with self._cond:
            self._active = max(0, self._active - 1)
            if self._active == 0:
                self._cond.notify_all()
                self._arm_idle_timer()

    def restore(
        self, img: Image.Image, strength: float = 1.0, only_center: bool = False
    ) -> Image.Image:
        device, model, helper = self.acquire()
        try:
            return self._restore(img, device, model, helper, strength, only_center)
        finally:
            self.release()

    def unload(self) -> bool:
        with self._cond:
            self._cancel_idle_timer()
            if self._model is None or self._active:
                return False
            self._unload()
            return True

    def close(self) -> None:
        with self._cond:
            self._closed = True
            self._cancel_idle_timer()
            while self._active:
                self._cond.wait()
            self._unload()

    def _load(self) -> None:
        self._loading = True
        try:
            if not os.path.isfile(self.model_path):
                raise FileNotFoundError(
                    f"GFPGAN weights not found at {self.model_path}. "
                    "Set FACE_MODEL_PATH to a GFPGANv1.4.pth file."
                )

            import cv2
            import torch
            from facexlib.utils.face_restoration_helper import FaceRestoreHelper
            from facexlib.utils.misc import img2tensor
            from torchvision.transforms.functional import normalize

            from facerestore.gfpgan_clean_arch import GFPGANv1Clean

            device = "cuda" if torch.cuda.is_available() else "cpu"
            print(f"Loading GFPGAN on {device} ... (first run may fetch facexlib weights)")

            state = torch.load(self.model_path, map_location="cpu")
            for key in ("params_ema", "params-ema", "params"):
                if key in state:
                    state = state[key]
                    break

            model = GFPGANv1Clean(state).eval().to(device)
            helper = FaceRestoreHelper(
                upscale_factor=1,
                face_size=512,
                crop_ratio=(1, 1),
                det_model="retinaface_resnet50",
                save_ext="png",
                use_parse=True,
                device=device,
                model_rootpath=self.aux_dir,
            )

            self._cv2 = cv2
            self._torch = torch
            self._img2tensor = img2tensor
            self._normalize = normalize
            self._device = device
            self._model = model
            self._helper = helper
            print("GFPGAN ready.")
        finally:
            self._loading = False

    def _restore(self, img, device, model, helper, strength, only_center):
        cv2 = self._cv2
        strength = max(0.0, min(1.0, float(strength)))
        has_alpha = img.mode in ("RGBA", "LA") or (
            img.mode == "P" and "transparency" in img.info
        )
        alpha = img.convert("RGBA").getchannel("A") if has_alpha else None

        bgr = cv2.cvtColor(np.array(img.convert("RGB")), cv2.COLOR_RGB2BGR)
        with INFERENCE_LOCK, self._torch.inference_mode():
            helper.read_image(bgr)
            try:
                faces = helper.get_face_landmarks_5(
                    only_center_face=only_center,
                    resize=640,
                    eye_dist_threshold=5,
                )
                if faces == 0:
                    return img.copy()

                helper.align_warp_face()
                for cropped in helper.cropped_faces:
                    x = self._img2tensor(
                        cropped.astype(np.float32) / 255.0,
                        bgr2rgb=True,
                        float32=True,
                    )
                    self._normalize(x, (0.5, 0.5, 0.5), (0.5, 0.5, 0.5), inplace=True)
                    x = x.unsqueeze(0).to(device)
                    restored_face = _tensor2img(
                        model(x)[0], rgb2bgr=True, min_max=(-1.0, 1.0)
                    )
                    helper.add_restored_face(restored_face)
                helper.get_inverse_affine(None)
                restored = helper.paste_faces_to_input_image()
            finally:
                # Never leave partial face state behind for the next request.
                helper.clean_all()

        if strength < 0.999:
            restored = cv2.addWeighted(bgr, 1.0 - strength, restored, strength, 0)

        out = Image.fromarray(cv2.cvtColor(restored, cv2.COLOR_BGR2RGB))
        if alpha is not None:
            out = out.convert("RGBA")
            out.putalpha(alpha)
        return out

    def _unload(self) -> bool:
        if self._model is None:
            return False
        torch, device = self._torch, self._device
        self._model = self._helper = self._device = self._torch = None
        self._cv2 = self._img2tensor = self._normalize = None
        gc.collect()
        if torch is not None and device == "cuda":
            torch.cuda.empty_cache()
        print("GFPGAN unloaded.")
        return True

    def _cancel_idle_timer(self) -> None:
        if self._idle_timer is not None:
            self._idle_timer.cancel()
            self._idle_timer = None

    def _arm_idle_timer(self) -> None:
        if self.idle_timeout <= 0 or self._model is None:
            return
        self._cancel_idle_timer()
        timer = threading.Timer(self.idle_timeout, self._idle_unload)
        timer.daemon = True
        timer.start()
        self._idle_timer = timer

    def _idle_unload(self) -> None:
        with self._cond:
            self._idle_timer = None
            if self._active == 0 and self._model is not None:
                self._unload()


FACE = FaceRestoreManager(FACE_MODEL_PATH, FACE_AUX_DIR, MODEL_IDLE_TIMEOUT)


def _model_tag(path: str) -> str:
    """Name + size, so replacing a weights file invalidates its cache."""
    try:
        return f"{os.path.basename(path)}:{os.path.getsize(path)}"
    except OSError:
        return os.path.basename(path)


FACE_TAG = _model_tag(FACE_MODEL_PATH)


def image_key(kind: str, img: Image.Image, **params) -> str:
    """Content + settings fingerprint for one processed image."""
    h = hashlib.sha256()
    h.update(f"v{CACHE_VERSION}|{kind}|".encode())
    h.update(f"{img.mode}:{img.width}x{img.height}|".encode())
    for key in sorted(params):
        h.update(f"{key}={params[key]!r}|".encode())
    h.update(img.tobytes())
    return h.hexdigest()


def _cache_file(kind: str, key: str) -> str:
    return os.path.join(CACHE_DIR, kind, key + ".png")


def read_cache(kind: str, key: str) -> Optional[bytes]:
    try:
        with open(_cache_file(kind, key), "rb") as f:
            return f.read()
    except OSError:
        return None


def write_cache(kind: str, key: str, data: bytes) -> None:
    path = _cache_file(kind, key)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + f".{uuid.uuid4().hex}.tmp"
    try:
        with open(tmp, "wb") as f:
            f.write(data)
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            try:
                os.remove(tmp)
            except OSError:
                pass


def output_path(name: str, suffix: str, key: str) -> str:
    """Readable copy in ./output: <source>.<suffix>.<key6>.png"""
    stem = os.path.splitext(os.path.basename(name or ""))[0]
    stem = re.sub(r"[^A-Za-z0-9._ -]+", "_", stem).strip(" ._")[:60] or "image"
    return os.path.join(OUTPUT_DIR, f"{stem}.{suffix}.{key[:6]}.png")


def save_output(path: str, img: Image.Image) -> None:
    """Write once: an existing file for the same key is left untouched."""
    if os.path.exists(path):
        return
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + f".{uuid.uuid4().hex}.tmp"
    try:
        img.save(tmp, "PNG")
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            try:
                os.remove(tmp)
            except OSError:
                pass


def mask_png(img: Image.Image) -> tuple[bytes, str, bool]:
    """Return (white+alpha mask PNG, cache key, was cached). No model on a hit."""
    key = image_key("mask", img, model=MODEL_ID)
    data = read_cache("mask", key)
    if data is not None:
        return data, key, True
    mask = predict_mask(img)
    rgba = Image.new("RGBA", mask.size, (255, 255, 255, 255))
    rgba.putalpha(mask)
    buf = io.BytesIO()
    rgba.save(buf, "PNG")
    data = buf.getvalue()
    write_cache("mask", key, data)
    return data, key, False


def restore_png(
    img: Image.Image, strength: float, only_center: bool
) -> tuple[bytes, str, bool]:
    """Return (restored PNG, cache key, was cached). No model on a hit."""
    key = image_key(
        "restore",
        img,
        model=FACE_TAG,
        strength=round(float(strength), 4),
        center=bool(only_center),
    )
    data = read_cache("restore", key)
    if data is not None:
        return data, key, True
    result = restore_faces(img, strength=strength, only_center=only_center)
    buf = io.BytesIO()
    result.save(buf, "PNG")
    data = buf.getvalue()
    write_cache("restore", key, data)
    return data, key, False


def restore_faces(
    img: Image.Image, strength: float = 1.0, only_center: bool = False
) -> Image.Image:
    """Run GFPGAN on every detected face, keeping the rest of the image intact."""
    if not FACE.available:
        raise HTTPException(
            503,
            f"GFPGAN weights not found at {FACE_MODEL_PATH}. "
            "Place GFPGANv1.4.pth in ./models or set FACE_MODEL_PATH.",
        )
    return FACE.restore(img, strength=strength, only_center=only_center)


@asynccontextmanager
async def lifespan(app: FastAPI):
    # Nothing is loaded here: the first request for a model loads it.
    try:
        _migrate_workspace_files()
    except Exception as exc:  # keep the server usable if a legacy file is locked
        print(f"Workspace migration skipped: {exc}")
    print(f"Ready (model: {MODEL_ID}, loads on first request). http://127.0.0.1:{PORT}")
    yield
    MODEL.close()
    FACE.close()


app = FastAPI(lifespan=lifespan)

# Local folders are served read-only so the UI can load saved images by URL.
app.mount("/input", StaticFiles(directory=INPUT_DIR), name="input")
app.mount("/output", StaticFiles(directory=OUTPUT_DIR), name="output")
# New workspaces keep only workspace.json here; their images live in /output.
# This mount remains for old workspaces that still have bundled files.
app.mount("/workspaces", StaticFiles(directory=WORKSPACES_DIR), name="workspaces")


def _now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def _workspace_dir(kind: str, sid: Optional[str] = None) -> str:
    if kind not in WORKSPACE_KINDS:
        raise HTTPException(404, "Unknown workspace type")
    base = os.path.join(WORKSPACES_DIR, kind)
    if sid is None:
        return base
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,80}", sid):
        raise HTTPException(400, "Invalid workspace id")
    return os.path.join(base, sid)


def _read_manifest(root: str) -> Optional[dict]:
    path = os.path.join(root, "workspace.json")
    if not os.path.isfile(path):
        return None
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else None
    except (OSError, ValueError):
        return None


def _write_manifest(root: str, manifest: dict) -> None:
    path = os.path.join(root, "workspace.json")
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)
    os.replace(tmp, path)


def _existing_output_file(value: object) -> Optional[str]:
    """Normalize an output/... reference, returning it only if the file exists."""
    rel = str(value or "").replace("\\", "/").lstrip("/")
    parts = [part for part in rel.split("/") if part not in ("", ".")]
    if len(parts) < 2 or parts[0] != "output" or ".." in parts:
        return None
    full = os.path.join(OUTPUT_DIR, *parts[1:])
    return "/".join(parts) if os.path.isfile(full) else None


def _workspace_meta(kind: str, sid: str, manifest: dict) -> dict:
    root = _workspace_dir(kind, sid)
    graph = manifest.get("graph")
    nodes = graph.get("nodes") if isinstance(graph, dict) else None
    thumb_url = None
    saved_thumb = _existing_output_file(manifest.get("thumb"))
    if saved_thumb:
        thumb_url = "/" + saved_thumb
    if thumb_url is None:
        legacy_thumb = os.path.join(root, "thumb.png")
        if os.path.isfile(legacy_thumb):
            thumb_url = f"/workspaces/{kind}/{sid}/thumb.png"
    return {
        "id": sid,
        "name": manifest.get("name") or sid,
        "createdAt": manifest.get("createdAt"),
        "updatedAt": manifest.get("updatedAt"),
        "nodes": len(nodes) if isinstance(nodes, list) else 0,
        "thumb": thumb_url,
    }


def _is_upload(value: object) -> bool:
    return (
        value is not None
        and hasattr(value, "read")
        and hasattr(value, "filename")
    )


def _asset_ext(upload: UploadFile, mime: str) -> str:
    ext = os.path.splitext(upload.filename or "")[1].lower()
    if ext in IMAGE_SUFFIXES:
        return ext
    guessed = mimetypes.guess_extension((mime or "").split(";")[0].strip())
    if guessed and guessed.lower() in IMAGE_SUFFIXES:
        return guessed.lower()
    return ".png"


async def _save_shared_upload(
    upload: UploadFile, dest_dir: str, public_dir: str, ext: str
) -> str:
    """Store an upload by content hash under output/shared and return its URL path."""
    os.makedirs(dest_dir, exist_ok=True)
    tmp = os.path.join(dest_dir, f".{uuid.uuid4().hex}.tmp")
    digest = hashlib.sha256()
    try:
        with open(tmp, "wb") as out:
            while True:
                chunk = await upload.read(1024 * 1024)
                if not chunk:
                    break
                digest.update(chunk)
                out.write(chunk)
        name = digest.hexdigest() + ext
        dest = os.path.join(dest_dir, name)
        if os.path.exists(dest):
            os.remove(tmp)
        else:
            os.replace(tmp, dest)
        return f"{public_dir}/{name}"
    finally:
        try:
            await upload.close()
        finally:
            try:
                if os.path.exists(tmp):
                    os.remove(tmp)
            except OSError:
                pass


def _save_shared_bytes(
    data: bytes, dest_dir: str, public_dir: str, ext: str = ".png"
) -> str:
    """Content-addressed write for bytes that are already in memory."""
    os.makedirs(dest_dir, exist_ok=True)
    name = hashlib.sha256(data).hexdigest() + ext
    dest = os.path.join(dest_dir, name)
    if not os.path.exists(dest):
        tmp = dest + f".{uuid.uuid4().hex}.tmp"
        try:
            with open(tmp, "wb") as out:
                out.write(data)
            os.replace(tmp, dest)
        finally:
            if os.path.exists(tmp):
                os.remove(tmp)
    return f"{public_dir}/{name}"


async def _store_workspace(
    kind: str, sid: str, request: Request, create: bool
) -> dict:
    base = _workspace_dir(kind)
    root = _workspace_dir(kind, sid)
    if create and os.path.exists(root):
        raise HTTPException(409, "Workspace already exists")
    if not create and not os.path.isdir(root):
        raise HTTPException(404, "Workspace not found")

    old = None if create else _read_manifest(root)
    tmp = os.path.join(base, f".{sid}.{uuid.uuid4().hex[:8]}.tmp")
    os.makedirs(tmp, exist_ok=False)
    try:
        form = await request.form()
        raw = form.get("manifest")
        if not isinstance(raw, str):
            raise HTTPException(400, "Missing manifest")
        try:
            manifest = json.loads(raw)
        except ValueError:
            raise HTTPException(400, "Manifest is not valid JSON")
        if not isinstance(manifest, dict):
            raise HTTPException(400, "Manifest must be an object")
        if not isinstance(manifest.get("graph"), dict):
            raise HTTPException(400, "Manifest has no graph")

        # Source images and processed canvases live in the shared output pool.
        # Their file names are SHA-256 content hashes, so saving the same image
        # in a second template/project reuses the exact same file.
        assets = []
        for i, item in enumerate(manifest.get("assets") or []):
            if not isinstance(item, dict):
                continue
            upload = form.get(str(item.get("field") or ""))
            aid = str(item.get("id") or f"asset{i}")
            if not _is_upload(upload):
                existing = _existing_output_file(item.get("file"))
                if existing:
                    assets.append(
                        {
                            "id": aid,
                            "name": str(item.get("name") or aid),
                            "mime": str(item.get("mime") or "image/png"),
                            "file": existing,
                        }
                    )
                continue
            ext = _asset_ext(upload, str(item.get("mime") or ""))
            rel = await _save_shared_upload(
                upload,
                SHARED_ASSET_DIR,
                "output/shared/assets",
                ext,
            )
            assets.append(
                {
                    "id": aid,
                    "name": str(item.get("name") or aid),
                    "mime": upload.content_type or item.get("mime") or "image/png",
                    "file": rel,
                }
            )

        masks = []
        for i, item in enumerate(manifest.get("masks") or []):
            if not isinstance(item, dict):
                continue
            upload = form.get(str(item.get("field") or ""))
            node_id = str(item.get("nodeId") or f"node{i}")
            if not _is_upload(upload):
                existing = _existing_output_file(item.get("file"))
                if existing:
                    masks.append({"nodeId": node_id, "file": existing})
                continue
            rel = await _save_shared_upload(
                upload,
                SHARED_MASK_DIR,
                "output/shared/masks",
                ".png",
            )
            masks.append({"nodeId": node_id, "file": rel})

        restores = []
        for i, item in enumerate(manifest.get("restores") or []):
            if not isinstance(item, dict):
                continue
            upload = form.get(str(item.get("field") or ""))
            node_id = str(item.get("nodeId") or f"node{i}")
            if not _is_upload(upload):
                existing = _existing_output_file(item.get("file"))
                if existing:
                    restores.append(
                        {
                            "nodeId": node_id,
                            "strength": item.get("strength"),
                            "onlyCenter": bool(item.get("onlyCenter")),
                            "file": existing,
                        }
                    )
                continue
            rel = await _save_shared_upload(
                upload,
                SHARED_RESTORE_DIR,
                "output/shared/restores",
                ".png",
            )
            restores.append(
                {
                    "nodeId": node_id,
                    "strength": item.get("strength"),
                    "onlyCenter": bool(item.get("onlyCenter")),
                    "file": rel,
                }
            )

        thumb = form.get("thumb")
        if _is_upload(thumb):
            manifest["thumb"] = await _save_shared_upload(
                thumb,
                SHARED_THUMB_DIR,
                "output/shared/thumbs",
                ".png",
            )
        else:
            existing_thumb = _existing_output_file((old or {}).get("thumb"))
            if existing_thumb:
                manifest["thumb"] = existing_thumb
            else:
                legacy_thumb = os.path.join(root, "thumb.png")
                if os.path.isfile(legacy_thumb):
                    with open(legacy_thumb, "rb") as f:
                        data = f.read()
                    manifest["thumb"] = _save_shared_bytes(
                        data,
                        SHARED_THUMB_DIR,
                        "output/shared/thumbs",
                    )

        name = str(manifest.get("name") or "").strip()[:120] or "Untitled"
        now = _now()
        manifest["id"] = sid
        manifest["name"] = name
        manifest["assets"] = assets
        manifest["masks"] = masks
        manifest["restores"] = restores
        manifest["createdAt"] = (old or {}).get("createdAt") or now
        manifest["updatedAt"] = now
        manifest.setdefault("version", 1)
        _write_manifest(tmp, manifest)

        if os.path.isdir(root):
            shutil.rmtree(root)
        # On Windows, antivirus can briefly hold freshly written files and make
        # a directory rename fail with WinError 5. Copying is read-only safe.
        promoted = False
        for attempt in range(4):
            try:
                os.replace(tmp, root)
                promoted = True
                break
            except PermissionError:
                if attempt < 3:
                    time.sleep(0.2)
        if not promoted:
            shutil.copytree(tmp, root)
        return _workspace_meta(kind, sid, manifest)
    finally:
        if os.path.isdir(tmp):
            shutil.rmtree(tmp, ignore_errors=True)


def _migrate_workspace_files() -> None:
    """Move images from legacy per-workspace folders into output/shared."""
    for kind in WORKSPACE_KINDS:
        base = _workspace_dir(kind)
        try:
            names = os.listdir(base)
        except OSError:
            continue
        for sid in names:
            if sid.startswith("."):
                continue
            root = os.path.join(base, sid)
            if not os.path.isdir(root):
                continue
            manifest = _read_manifest(root)
            if not manifest:
                continue
            root_abs = os.path.abspath(root)
            changed = False
            old_files = []
            for key, dest_dir, public_dir in (
                ("assets", SHARED_ASSET_DIR, "output/shared/assets"),
                ("masks", SHARED_MASK_DIR, "output/shared/masks"),
                ("restores", SHARED_RESTORE_DIR, "output/shared/restores"),
            ):
                items = manifest.get(key)
                if not isinstance(items, list):
                    continue
                for item in items:
                    if not isinstance(item, dict):
                        continue
                    rel = str(item.get("file") or "").replace("\\", "/").lstrip("/")
                    if not rel or rel.startswith("output/"):
                        continue
                    src = os.path.abspath(os.path.join(root, *rel.split("/")))
                    if not src.startswith(root_abs + os.sep) or not os.path.isfile(src):
                        continue
                    ext = os.path.splitext(src)[1].lower()
                    if ext not in IMAGE_SUFFIXES:
                        ext = ".png"
                    try:
                        with open(src, "rb") as f:
                            data = f.read()
                        item["file"] = _save_shared_bytes(
                            data, dest_dir, public_dir, ext
                        )
                    except OSError:
                        continue
                    changed = True
                    old_files.append(src)

            legacy_thumb = os.path.join(root, "thumb.png")
            saved_thumb = manifest.get("thumb")
            if not (
                isinstance(saved_thumb, str)
                and saved_thumb.replace("\\", "/").lstrip("/").startswith("output/")
            ) and os.path.isfile(legacy_thumb):
                try:
                    with open(legacy_thumb, "rb") as f:
                        data = f.read()
                    manifest["thumb"] = _save_shared_bytes(
                        data,
                        SHARED_THUMB_DIR,
                        "output/shared/thumbs",
                    )
                except OSError:
                    pass
                else:
                    changed = True
                    old_files.append(legacy_thumb)

            if not changed:
                continue
            try:
                _write_manifest(root, manifest)
            except OSError:
                continue
            for src in old_files:
                try:
                    os.remove(src)
                except OSError:
                    pass
            for sub in ("assets", "masks", "restores"):
                try:
                    os.rmdir(os.path.join(root, sub))
                except OSError:
                    pass


@app.get("/api/workspaces/{kind}")
def list_workspaces(kind: str):
    """Every saved template/project, newest first."""
    base = _workspace_dir(kind)
    items = []
    try:
        names = os.listdir(base)
    except OSError:
        return items
    for sid in names:
        if sid.startswith("."):
            continue
        root = os.path.join(base, sid)
        if not os.path.isdir(root):
            continue
        manifest = _read_manifest(root)
        if manifest:
            items.append(_workspace_meta(kind, sid, manifest))
    items.sort(key=lambda item: item.get("updatedAt") or "", reverse=True)
    return items


@app.post("/api/workspaces/{kind}")
async def create_workspace(kind: str, request: Request):
    """Save the current graph as a new template/project."""
    _workspace_dir(kind)
    sid = uuid.uuid4().hex[:12]
    return await _store_workspace(kind, sid, request, create=True)


@app.get("/api/workspaces/{kind}/{sid}")
def get_workspace(kind: str, sid: str):
    """Full graph manifest, including the output/ path of every saved image."""
    root = _workspace_dir(kind, sid)
    manifest = _read_manifest(root)
    if not manifest:
        raise HTTPException(404, "Workspace not found")
    manifest["id"] = sid
    meta = _workspace_meta(kind, sid, manifest)
    manifest["thumb"] = meta["thumb"]
    return manifest


@app.put("/api/workspaces/{kind}/{sid}")
async def update_workspace(kind: str, sid: str, request: Request):
    """Overwrite a project in place (the current project's Save action)."""
    return await _store_workspace(kind, sid, request, create=False)


@app.patch("/api/workspaces/{kind}/{sid}")
def rename_workspace(kind: str, sid: str, payload: dict = Body(...)):
    root = _workspace_dir(kind, sid)
    manifest = _read_manifest(root)
    if not manifest:
        raise HTTPException(404, "Workspace not found")
    name = str(payload.get("name") or "").strip()[:120]
    if not name:
        raise HTTPException(400, "Name is required")
    manifest["name"] = name
    manifest["updatedAt"] = _now()
    _write_manifest(root, manifest)
    return _workspace_meta(kind, sid, manifest)


@app.delete("/api/workspaces/{kind}/{sid}")
def delete_workspace(kind: str, sid: str):
    root = _workspace_dir(kind, sid)
    if not os.path.isdir(root):
        raise HTTPException(404, "Workspace not found")
    shutil.rmtree(root)
    return {"deleted": True, "id": sid}


def fetch_url(url: str) -> Image.Image:
    p = urlparse(url)
    if p.scheme not in ("http", "https") or not p.netloc:
        raise HTTPException(400, "URL must start with http:// or https://")
    try:
        r = requests.get(
            url,
            timeout=20,
            stream=True,
            headers={"User-Agent": "Mozilla/5.0 bgremover"},
        )
        r.raise_for_status()
        buf = io.BytesIO()
        for chunk in r.iter_content(65536):
            buf.write(chunk)
            if buf.tell() > MAX_DOWNLOAD_BYTES:
                raise HTTPException(413, "Image is larger than 30 MB")
        buf.seek(0)
        return Image.open(buf)
    except HTTPException:
        raise
    except Exception as e:
        raise HTTPException(400, f"Could not download image: {e}")


def load_input(file: Optional[UploadFile], url: str) -> Image.Image:
    if file is not None and file.filename:
        try:
            img = Image.open(io.BytesIO(file.file.read()))
        except Exception:
            raise HTTPException(400, "Could not read that file as an image")
    elif url.strip():
        img = fetch_url(url.strip())
    else:
        raise HTTPException(400, "Provide an image or a URL")

    img = ImageOps.exif_transpose(img)
    if max(img.size) > MAX_PREVIEW_SIDE:
        img.thumbnail((MAX_PREVIEW_SIDE, MAX_PREVIEW_SIDE), Image.LANCZOS)
    return img


def limit_image(img: Image.Image, max_width: int = 0, max_side: int = 0) -> Image.Image:
    if max_side > 0 and max(img.size) > max_side:
        img.thumbnail((max_side, max_side), Image.LANCZOS)
    if max_width > 0 and img.width > max_width:
        h = round(img.height * max_width / img.width)
        img = img.resize((max_width, h), Image.LANCZOS)
    return img


def predict_mask(img: Image.Image) -> Image.Image:
    """Load the model on first use, run BiRefNet, return a soft grayscale mask."""
    rgb = img.convert("RGB")
    device, tfm = MODEL.acquire()
    try:
        x = tfm(rgb).unsqueeze(0).to(device)
        pred = MODEL.predict(x)
    finally:
        MODEL.release()
    mask = Image.fromarray((pred.clamp(0, 1).numpy() * 255).astype(np.uint8), "L")
    return mask.resize(rgb.size, Image.LANCZOS)


@app.post("/api/remove")
def api_remove(
    file: Optional[UploadFile] = File(None),
    url: str = Form(""),
    max_width: int = Form(0),
    max_side: int = Form(0),
    name: str = Form(""),
):
    img = limit_image(load_input(file, url), max_width=max_width, max_side=max_side)
    t0 = time.time()
    data, key, cached = mask_png(img)
    alpha = Image.open(io.BytesIO(data)).convert("RGBA").getchannel("A")
    result = img.convert("RGB")
    result.putalpha(alpha)

    out = output_path(name, "rembg", key)
    save_output(out, result)

    buf = io.BytesIO()
    result.save(buf, "PNG")
    return Response(
        content=buf.getvalue(),
        media_type="image/png",
        headers={
            "X-Process-Time": f"{time.time() - t0:.2f}",
            "X-Cache": "hit" if cached else "miss",
            "X-Output": os.path.basename(out),
            "Cache-Control": "no-store",
        },
    )


@app.post("/api/mask")
def api_mask(
    file: Optional[UploadFile] = File(None),
    url: str = Form(""),
    max_width: int = Form(0),
    max_side: int = Form(0),
    name: str = Form(""),
):
    """
    Return a soft alpha mask at the input resolution.

    RGB is white and the mask lives in the alpha channel, so the browser can
    apply it with canvas `destination-in` without any luminance conversion.
    The mask is cached by image content + settings; a hit skips BiRefNet.
    """
    img = limit_image(load_input(file, url), max_width=max_width, max_side=max_side)
    t0 = time.time()
    data, key, cached = mask_png(img)

    # Also drop a ready-to-use cutout in ./output for the file browser.
    out = output_path(name, "rembg", key)
    if not os.path.exists(out):
        alpha = Image.open(io.BytesIO(data)).convert("RGBA").getchannel("A")
        cut = img.convert("RGB")
        cut.putalpha(alpha)
        save_output(out, cut)

    return Response(
        content=data,
        media_type="image/png",
        headers={
            "X-Process-Time": f"{time.time() - t0:.2f}",
            "X-Cache": "hit" if cached else "miss",
            "X-Output": os.path.basename(out),
            "Cache-Control": "no-store",
        },
    )


@app.post("/api/restore")
def api_restore(
    file: Optional[UploadFile] = File(None),
    url: str = Form(""),
    strength: float = Form(1.0),
    only_center: bool = Form(False),
    max_side: int = Form(0),
    name: str = Form(""),
):
    """
    Restore faces with GFPGAN.

    `strength` blends the restored faces back over the original (0 = off,
    1 = full restoration). `only_center` restores just the most central face.
    The output keeps the input resolution and alpha channel. Results are
    cached by input content + strength + mode, so repeats skip GFPGAN.
    """
    img = limit_image(load_input(file, url), max_side=max_side)
    t0 = time.time()
    data, key, cached = restore_png(img, strength=strength, only_center=only_center)

    out = output_path(name, "restored", key)
    if not os.path.exists(out):
        result = Image.open(io.BytesIO(data))
        result.load()
        save_output(out, result)

    return Response(
        content=data,
        media_type="image/png",
        headers={
            "X-Process-Time": f"{time.time() - t0:.2f}",
            "X-Cache": "hit" if cached else "miss",
            "X-Output": os.path.basename(out),
            "Cache-Control": "no-store",
        },
    )


@app.get("/api/fetch")
def api_fetch(url: str):
    """Download a URL server-side and re-encode it for a reliable browser preview."""
    img = ImageOps.exif_transpose(fetch_url(url))
    if max(img.size) > MAX_PREVIEW_SIDE:
        img.thumbnail((MAX_PREVIEW_SIDE, MAX_PREVIEW_SIDE), Image.LANCZOS)

    has_alpha = img.mode in ("RGBA", "LA") or (
        img.mode == "P" and "transparency" in img.info
    )
    buf = io.BytesIO()
    if has_alpha:
        img.convert("RGBA").save(buf, "PNG")
        media_type = "image/png"
    else:
        img.convert("RGB").save(buf, "JPEG", quality=95)
        media_type = "image/jpeg"
    return Response(
        content=buf.getvalue(),
        media_type=media_type,
        headers={"Cache-Control": "no-store"},
    )


@app.get("/api/files")
def api_files():
    """Images available in the local ./input and ./output folders."""

    def listing(folder: str, kind: str):
        items = []
        try:
            names = os.listdir(folder)
        except OSError:
            return items
        for n in names:
            p = os.path.join(folder, n)
            if not n.lower().endswith(IMAGE_SUFFIXES) or not os.path.isfile(p):
                continue
            try:
                st = os.stat(p)
            except OSError:
                continue
            items.append(
                {
                    "name": n,
                    "kind": kind,
                    "url": f"/{kind}/{n}",
                    "size": st.st_size,
                    "mtime": st.st_mtime,
                }
            )
        items.sort(key=lambda x: x["mtime"], reverse=True)
        return items[:200]

    return {
        "input": listing(INPUT_DIR, "input"),
        "output": listing(OUTPUT_DIR, "output"),
    }


@app.get("/", include_in_schema=False)
def index():
    return HTMLResponse("HELLO")


@app.get("/health")
def health():
    return {
        "ok": True,
        "model": MODEL_ID,
        "device": MODEL.device,
        "loaded": MODEL.loaded,
        "status": MODEL.status,
        "face_model": os.path.basename(FACE_MODEL_PATH),
        "face_device": FACE.device,
        "face_loaded": FACE.loaded,
        "face_status": FACE.status,
        "face_available": FACE.available,
    }


@app.post("/api/unload")
def api_unload():
    """Free both models. A model is skipped (unloaded=false) while still in use."""
    return {
        "unloaded": MODEL.unload(),
        "status": MODEL.status,
        "face_unloaded": FACE.unload(),
        "face_status": FACE.status,
    }


if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=PORT)
