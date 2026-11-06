# Quby Render Queue 1.3 - job settings applier
# Runs inside Blender (background). Never saves the .blend file, so every
# change here is non-destructive to the artist's project.
# Compatible with Blender 3.x, 4.x and 5.x.

import sys
import json
import math

TAG = "QRQ"


def log(msg):
    try:
        print("%s: %s" % (TAG, msg), flush=True)
    except Exception:
        pass


def warn(msg):
    try:
        print("%s-WARN: %s" % (TAG, msg), flush=True)
    except Exception:
        pass


def post_argv():
    a = list(sys.argv)
    if "--" in a:
        a = a[a.index("--") + 1:]
    return a


def argval(flag):
    a = post_argv()
    if flag in a:
        i = a.index(flag)
        if i + 1 < len(a):
            return a[i + 1]
    return None


# --------------------------------------------------------------------------
# small safe setters
# --------------------------------------------------------------------------

def enum_values(owner, attr):
    try:
        return [i.identifier for i in owner.bl_rna.properties[attr].enum_items]
    except Exception:
        return None


def set_enum(owner, attr, value, label=None):
    label = label or attr
    if owner is None or value is None or value == "":
        return False
    if not hasattr(owner, attr):
        warn("%s is not available in this Blender version" % label)
        return False
    valid = enum_values(owner, attr)
    if valid and value not in valid:
        warn("%s = '%s' is not supported here. Available: %s"
             % (label, value, ", ".join(valid[:16])))
        return False
    try:
        setattr(owner, attr, value)
        log("%s = %s" % (label, value))
        return True
    except Exception as e:
        warn("could not set %s: %s" % (label, e))
        return False


def set_val(owner, attr, value, label=None):
    label = label or attr
    if owner is None or value is None:
        return False
    if not hasattr(owner, attr):
        warn("%s is not available in this Blender version" % label)
        return False
    try:
        setattr(owner, attr, value)
        log("%s = %s" % (label, value))
        return True
    except Exception as e:
        warn("could not set %s: %s" % (label, e))
        return False


def get_val(owner, attr, fallback=None):
    try:
        return getattr(owner, attr)
    except Exception:
        return fallback


# --------------------------------------------------------------------------
# fps helpers
# --------------------------------------------------------------------------

NTSC = {23.976: (24, 1.001), 23.98: (24, 1.001),
        29.97: (30, 1.001), 59.94: (60, 1.001),
        47.952: (48, 1.001), 119.88: (120, 1.001)}


def fps_pair(target):
    """Return (fps_int, fps_base) for a float fps, Blender style."""
    for k, v in NTSC.items():
        if abs(target - k) < 0.01:
            return v
    if abs(target - round(target)) < 1e-6:
        return (int(round(target)), 1.0)
    # generic fractional fps: smallest rational that hits the target
    try:
        from fractions import Fraction
        frac = Fraction(target).limit_denominator(1000)
        if frac.numerator >= 1:
            return (int(frac.numerator), float(frac.denominator))
    except Exception:
        pass
    return (int(round(target * 1000.0)), 1000.0)


def ratio_within(value, limit=900):
    """Best fraction approximating `value` with numerator and denominator <= limit."""
    from fractions import Fraction
    best = None
    d = limit
    while d >= 1:
        frac = Fraction(value).limit_denominator(d)
        if 1 <= frac.numerator <= limit and 1 <= frac.denominator <= limit:
            best = frac
            break
        d = min(d - 1, int(d * limit / max(frac.numerator, 1)))
    if best is None:
        best = Fraction(1, 1)
    return int(best.numerator), int(best.denominator)


def effective_fps(scene):
    base = get_val(scene.render, "fps_base", 1.0) or 1.0
    try:
        return float(scene.render.fps) / float(base)
    except Exception:
        return float(get_val(scene.render, "fps", 24))


# --------------------------------------------------------------------------
# retiming
# --------------------------------------------------------------------------

def iter_fcurves(action):
    """Yield every fcurve of an action, legacy and slotted (4.4+) layouts."""
    seen = set()
    try:
        for layer in getattr(action, "layers", []):
            for strip in getattr(layer, "strips", []):
                for bag in getattr(strip, "channelbags", []):
                    for fc in getattr(bag, "fcurves", []):
                        p = fc.as_pointer()
                        if p not in seen:
                            seen.add(p)
                            yield fc
    except Exception:
        pass
    try:
        for fc in getattr(action, "fcurves", []):
            p = fc.as_pointer()
            if p not in seen:
                seen.add(p)
                yield fc
    except Exception:
        pass


def scale_fcurve(fc, k):
    kps = getattr(fc, "keyframe_points", None)
    if kps is None:
        return 0
    n = len(kps)
    if n == 0:
        return 0
    done = False
    try:
        import numpy as np
        for attr in ("co", "handle_left", "handle_right"):
            buf = np.empty(n * 2, dtype=np.float32)
            kps.foreach_get(attr, buf)
            buf[0::2] *= k
            kps.foreach_set(attr, buf)
        done = True
    except Exception:
        done = False
    if not done:
        for kp in kps:
            try:
                kp.co.x *= k
                kp.handle_left.x *= k
                kp.handle_right.x *= k
            except Exception:
                pass
    # fcurve modifiers live in frame space too
    for m in getattr(fc, "modifiers", []):
        for attr in ("frame_start", "frame_end", "start_frame", "end_frame"):
            v = get_val(m, attr)
            if isinstance(v, float) or isinstance(v, int):
                try:
                    setattr(m, attr, v * k)
                except Exception:
                    pass
        # noise / cycles style modifiers use a period in frames
        for attr in ("scale", "phase", "period"):
            v = get_val(m, attr)
            if isinstance(v, float):
                try:
                    setattr(m, attr, v * k)
                except Exception:
                    pass
    try:
        fc.update()
    except Exception:
        pass
    return n


def all_animdata(bpy):
    """Yield every animation_data block we can reach."""
    names = ("objects", "scenes", "materials", "node_groups", "worlds",
             "shape_keys", "cameras", "lights", "meshes", "curves",
             "armatures", "textures", "speakers", "linestyles", "masks",
             "movieclips", "particles", "collections", "images",
             "grease_pencils", "volumes", "lattices", "metaballs")
    for name in names:
        coll = getattr(bpy.data, name, None)
        if coll is None:
            continue
        for item in coll:
            ad = get_val(item, "animation_data")
            if ad is not None:
                yield ad
            nt = get_val(item, "node_tree")
            if nt is not None:
                ad2 = get_val(nt, "animation_data")
                if ad2 is not None:
                    yield ad2


def retime_keyframes(bpy, k):
    n_curves = 0
    n_keys = 0
    for action in bpy.data.actions:
        for fc in iter_fcurves(action):
            n_keys += scale_fcurve(fc, k)
            n_curves += 1
    # drivers are fcurves too, but they are not stored in bpy.data.actions
    seen_ad = set()
    for ad in all_animdata(bpy):
        try:
            p = ad.as_pointer()
        except Exception:
            p = None
        if p is not None:
            if p in seen_ad:
                continue
            seen_ad.add(p)
        for fc in getattr(ad, "drivers", []):
            n_keys += scale_fcurve(fc, k)
            n_curves += 1
        # NLA strips
        for track in getattr(ad, "nla_tracks", []):
            for strip in getattr(track, "strips", []):
                scale_nla_strip(strip, k)
    log("retimed %d f-curves / %d keyframes (factor %.6f)" % (n_curves, n_keys, k))


def scale_nla_strip(strip, k):
    vals = {}
    for attr in ("frame_start", "frame_end", "action_frame_start", "action_frame_end"):
        v = get_val(strip, attr)
        if v is not None:
            vals[attr] = v * k
    # end first when stretching, start first when shrinking
    order = ["frame_end", "frame_start"] if k > 1.0 else ["frame_start", "frame_end"]
    for attr in order:
        if attr not in vals:
            continue
        ui_attr = attr + "_ui"
        if hasattr(strip, ui_attr):
            try:
                setattr(strip, ui_attr, vals[attr])
                continue
            except Exception:
                pass
        try:
            setattr(strip, attr, vals[attr])
        except Exception:
            pass
    for attr in ("action_frame_start", "action_frame_end"):
        if attr in vals:
            try:
                setattr(strip, attr, vals[attr])
            except Exception:
                pass


def retime_grease_pencil(bpy, k):
    for coll_name in ("grease_pencils", "grease_pencils_v3"):
        coll = getattr(bpy.data, coll_name, None)
        if coll is None:
            continue
        for gp in coll:
            for layer in getattr(gp, "layers", []):
                frames = list(getattr(layer, "frames", []))
                # scale from the end so we never collide with an existing frame
                if k > 1.0:
                    frames = frames[::-1]
                for f in frames:
                    v = get_val(f, "frame_number")
                    if v is None:
                        continue
                    try:
                        f.frame_number = int(round(v * k))
                    except Exception:
                        pass


def retime_caches(bpy, k):
    def do_cache(pc):
        if pc is None:
            return
        for attr in ("frame_start", "frame_end"):
            v = get_val(pc, attr)
            if v is None:
                continue
            try:
                setattr(pc, attr, int(round(v * k)))
            except Exception:
                pass

    for obj in getattr(bpy.data, "objects", []):
        for mod in getattr(obj, "modifiers", []):
            do_cache(get_val(mod, "point_cache"))
            settings = get_val(mod, "settings")
            if settings is not None:
                do_cache(get_val(settings, "point_cache"))
            dom = get_val(mod, "domain_settings")
            if dom is not None:
                do_cache(get_val(dom, "point_cache"))
                for attr in ("cache_frame_start", "cache_frame_end"):
                    v = get_val(dom, attr)
                    if v is not None:
                        try:
                            setattr(dom, attr, int(round(v * k)))
                        except Exception:
                            pass
        for psys in getattr(obj, "particle_systems", []):
            do_cache(get_val(psys, "point_cache"))
            st = get_val(psys, "settings")
            if st is not None:
                for attr in ("frame_start", "frame_end", "lifetime", "lifetime_random"):
                    v = get_val(st, attr)
                    if isinstance(v, (int, float)) and attr != "lifetime_random":
                        try:
                            setattr(st, attr, v * k)
                        except Exception:
                            pass
        sb = get_val(obj, "soft_body")
        if sb is not None:
            do_cache(get_val(sb, "point_cache"))
    for scene in getattr(bpy.data, "scenes", []):
        rbw = get_val(scene, "rigidbody_world")
        if rbw is not None:
            do_cache(get_val(rbw, "point_cache"))


def vse_strips(scene):
    se = get_val(scene, "sequence_editor")
    if se is None:
        return []
    for attr in ("strips_all", "sequences_all", "strips", "sequences"):
        coll = get_val(se, attr)
        if coll is not None:
            try:
                return list(coll)
            except Exception:
                pass
    return []


def retime_vse(scene, k):
    strips = vse_strips(scene)
    if not strips:
        return
    # process in reverse timeline order when stretching to avoid clamping
    try:
        strips.sort(key=lambda s: get_val(s, "frame_final_start", 0) or 0,
                    reverse=(k > 1.0))
    except Exception:
        pass
    n = 0
    for s in strips:
        try:
            start = get_val(s, "frame_start")
            dur = get_val(s, "frame_final_duration")
            off_s = get_val(s, "frame_offset_start", 0) or 0
            off_e = get_val(s, "frame_offset_end", 0) or 0
            still_s = get_val(s, "frame_still_start", 0) or 0
            still_e = get_val(s, "frame_still_end", 0) or 0
            if start is not None:
                set_silent(s, "frame_start", start * k)
            for attr, v in (("frame_offset_start", off_s), ("frame_offset_end", off_e),
                            ("frame_still_start", still_s), ("frame_still_end", still_e)):
                if v:
                    set_silent(s, attr, v * k)
            if dur:
                set_silent(s, "frame_final_duration", max(1, int(round(dur * k))))
            n += 1
        except Exception:
            pass
    log("retimed %d sequencer strips" % n)


def set_silent(owner, attr, value):
    try:
        setattr(owner, attr, value)
        return True
    except Exception:
        return False


def retime_scene_frames(scene, k):
    r = scene.render
    start = get_val(scene, "frame_start", 1)
    end = get_val(scene, "frame_end", 1)
    new_end = int(round(end * k))
    new_start = int(round(start * k))
    if new_start < 0:
        new_start = 0
    if new_end < new_start:
        new_end = new_start
    # widen first so we never clamp
    if k > 1.0:
        set_silent(scene, "frame_end", new_end)
        set_silent(scene, "frame_start", new_start)
    else:
        set_silent(scene, "frame_start", new_start)
        set_silent(scene, "frame_end", new_end)
    cur = get_val(scene, "frame_current", new_start)
    set_silent(scene, "frame_current",
               min(max(int(round(cur * k)), new_start), new_end))
    for m in getattr(scene, "timeline_markers", []):
        v = get_val(m, "frame")
        if v is not None:
            set_silent(m, "frame", int(round(v * k)))
    # motion blur shutter is measured in frames: keep the real exposure time
    sh = get_val(r, "motion_blur_shutter")
    if isinstance(sh, float):
        set_silent(r, "motion_blur_shutter", max(0.0, min(2.0, sh * k)))
    cy = get_val(scene, "cycles")
    if cy is not None:
        sh2 = get_val(cy, "motion_blur_shutter")
        if isinstance(sh2, float):
            set_silent(cy, "motion_blur_shutter", max(0.0, min(2.0, sh2 * k)))
    log("frame range %d-%d -> %d-%d" % (start, end, new_start, new_end))


def apply_time_remap(scene, old_fps, new_fps):
    """Use Blender's own Time Remapping so animation keeps its real-world speed."""
    r = scene.render
    if not hasattr(r, "frame_map_old") or not hasattr(r, "frame_map_new"):
        return False
    try:
        ratio = old_fps / new_fps
        # compose with any time remap already set in the file
        prev_old = float(get_val(r, "frame_map_old", 100) or 100)
        prev_new = float(get_val(r, "frame_map_new", 100) or 100)
        if abs(prev_old - prev_new) > 1e-9:
            ratio = ratio * (prev_old / prev_new)
        old_map, new_map = ratio_within(ratio, 900)
        err = abs((old_map / float(new_map)) - ratio) / max(ratio, 1e-9)
        if err > 0.002:
            warn("this fps ratio cannot be expressed exactly by Blender's time "
                 "remap (%.3f%% off), using keyframe rescaling instead" % (err * 100.0))
            return False
        r.frame_map_old = old_map
        r.frame_map_new = new_map
        log("time remap %d/%d (animation keeps its original real-time speed)"
            % (old_map, new_map))
        return True
    except Exception as e:
        warn("time remap failed: %s" % e)
        return False


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------

def main():
    import bpy

    path = argval("--qrq-json")
    if not path:
        warn("no settings file given, rendering the file as-is")
        return
    try:
        with open(path, "r", encoding="utf-8") as f:
            cfg = json.load(f)
    except Exception as e:
        warn("could not read settings (%s), rendering the file as-is" % e)
        return

    # ---- scene -----------------------------------------------------------
    scene = bpy.context.scene
    want_scene = cfg.get("scene", "")
    if want_scene and want_scene in bpy.data.scenes:
        scene = bpy.data.scenes[want_scene]
        try:
            bpy.context.window.scene = scene
        except Exception:
            pass
        log("scene = %s" % want_scene)
    r = scene.render
    img = r.image_settings

    # ---- engine ----------------------------------------------------------
    engine = cfg.get("engine", "")
    if engine:
        set_enum(r, "engine", engine, "render engine")

    # ---- camera ----------------------------------------------------------
    cam = cfg.get("camera", "")
    if cam:
        ob = bpy.data.objects.get(cam)
        if ob is not None and ob.type == "CAMERA":
            scene.camera = ob
            log("camera = %s" % cam)
        else:
            warn("camera '%s' not found in this file" % cam)

    # ---- view layers -----------------------------------------------------
    only_layer = cfg.get("view_layer", "")
    if only_layer:
        found = False
        for vl in scene.view_layers:
            on = (vl.name == only_layer)
            set_silent(vl, "use", on)
            found = found or on
        if found:
            log("view layer = %s" % only_layer)
        else:
            warn("view layer '%s' not found" % only_layer)

    # ---- resolution ------------------------------------------------------
    res = cfg.get("resolution")
    if res and res.get("enabled"):
        set_val(r, "resolution_x", int(res.get("x", 1920)), "resolution x")
        set_val(r, "resolution_y", int(res.get("y", 1080)), "resolution y")
        set_val(r, "resolution_percentage", int(res.get("percentage", 100)),
                "resolution %")
        px = res.get("pixel_aspect")
        if px:
            set_silent(r, "pixel_aspect_x", 1.0)
            set_silent(r, "pixel_aspect_y", 1.0)

    # ---- colour management ----------------------------------------------
    col = cfg.get("color")
    if col and col.get("enabled"):
        vs = get_val(scene, "view_settings")
        ds = get_val(scene, "display_settings")
        if ds is not None:
            set_enum(ds, "display_device", col.get("display_device", ""), "display device")
        if vs is not None:
            set_enum(vs, "view_transform", col.get("view_transform", ""), "view transform")
            # the look list depends on the view transform, so set it after
            look = col.get("look", "")
            if look:
                if not set_enum(vs, "look", look, "look"):
                    # Blender prefixes looks with the transform name in 4.x
                    for candidate in enum_values(vs, "look") or []:
                        if candidate.split(" - ")[-1] == look.split(" - ")[-1]:
                            set_enum(vs, "look", candidate, "look")
                            break
            e = col.get("exposure")
            if e is not None:
                set_val(vs, "exposure", float(e), "exposure")
            g = col.get("gamma")
            if g is not None:
                set_val(vs, "gamma", float(g), "gamma")
        seq_cs = col.get("sequencer_colorspace", "")
        if seq_cs:
            ss = get_val(scene, "sequencer_colorspace_settings")
            if ss is not None:
                set_enum(ss, "name", seq_cs, "sequencer color space")
        out_cs = col.get("output_colorspace", "")
        if out_cs:
            lin = get_val(img, "linear_colorspace_settings")
            if lin is not None:
                set_enum(lin, "name", out_cs, "output color space")
            else:
                warn("output color space override needs Blender 4.0 or newer")

    # ---- file format -----------------------------------------------------
    fmt = cfg.get("format")
    if fmt and fmt.get("enabled"):
        ff = fmt.get("file_format", "")
        set_enum(img, "file_format", ff, "file format")
        set_enum(img, "color_mode", fmt.get("color_mode", ""), "color mode")
        set_enum(img, "color_depth", fmt.get("color_depth", ""), "color depth")
        # compression / quality
        comp = fmt.get("compression")
        if comp is not None and hasattr(img, "compression"):
            set_val(img, "compression", int(comp), "PNG compression")
        qual = fmt.get("quality")
        if qual is not None and hasattr(img, "quality"):
            set_val(img, "quality", int(qual), "quality")
        set_enum(img, "exr_codec", fmt.get("exr_codec", ""), "EXR codec")
        set_enum(img, "tiff_codec", fmt.get("tiff_codec", ""), "TIFF codec")
        if fmt.get("use_preview") is not None:
            set_silent(img, "use_preview", bool(fmt.get("use_preview")))

    ffm = cfg.get("ffmpeg")
    if ffm and ffm.get("enabled") and get_val(img, "file_format") == "FFMPEG":
        fobj = get_val(r, "ffmpeg")
        if fobj is None:
            warn("this build has no FFmpeg support")
        else:
            set_enum(fobj, "format", ffm.get("container", ""), "video container")
            set_enum(fobj, "codec", ffm.get("video_codec", ""), "video codec")
            set_enum(fobj, "audio_codec", ffm.get("audio_codec", ""), "audio codec")
            set_enum(fobj, "constant_rate_factor", ffm.get("crf", ""), "output quality")
            set_enum(fobj, "ffmpeg_preset", ffm.get("preset", ""), "encoding speed")
            gop = ffm.get("gopsize")
            if gop:
                set_val(fobj, "gopsize", int(gop), "keyframe interval")
            ab = ffm.get("audio_bitrate")
            if ab:
                set_val(fobj, "audio_bitrate", int(ab), "audio bitrate")
            if ffm.get("use_custom_bitrate"):
                set_silent(fobj, "constant_rate_factor", "NONE")
                set_val(fobj, "video_bitrate", int(ffm.get("video_bitrate", 6000)),
                        "video bitrate")
            if ffm.get("use_autosplit") is not None:
                set_silent(fobj, "use_autosplit", bool(ffm.get("use_autosplit")))

    # ---- post processing (compositor / sequencer) ------------------------
    post = cfg.get("post")
    if post and post.get("enabled"):
        set_val(r, "use_compositing", bool(post.get("use_compositing", True)),
                "use compositor")
        set_val(r, "use_sequencer", bool(post.get("use_sequencer", True)),
                "use video editor")
        if post.get("skip_3d"):
            n = 0
            for vl in scene.view_layers:
                if set_silent(vl, "use", False):
                    n += 1
            log("skipped the 3D render, disabled %d view layer(s)" % n)
            if not (post.get("use_compositing") or post.get("use_sequencer")):
                warn("skip 3D is on but both compositor and video editor are off, "
                     "the output will be empty")

    # ---- sampling / quality ---------------------------------------------
    q = cfg.get("quality")
    if q and q.get("enabled"):
        cy = get_val(scene, "cycles")
        engine_now = get_val(r, "engine", "")
        if cy is not None and engine_now == "CYCLES":
            s = q.get("samples")
            if s:
                set_val(cy, "samples", int(s), "cycles samples")
            th = q.get("adaptive_threshold")
            if th is not None:
                set_val(cy, "adaptive_threshold", float(th), "noise threshold")
            if q.get("use_denoise") is not None:
                set_val(cy, "use_denoising", bool(q.get("use_denoise")), "denoise")
            set_enum(cy, "denoiser", q.get("denoiser", ""), "denoiser")
        ee = get_val(scene, "eevee")
        if ee is not None and engine_now.startswith("BLENDER_EEVEE"):
            es = q.get("eevee_samples")
            if es:
                if hasattr(ee, "taa_render_samples"):
                    set_val(ee, "taa_render_samples", int(es), "eevee samples")
        mb = q.get("motion_blur")
        if mb in (0, 1):
            set_silent(r, "use_motion_blur", bool(mb))
            if cy is not None:
                set_silent(cy, "use_motion_blur", bool(mb))
        ft = q.get("film_transparent")
        if ft in (0, 1):
            set_silent(r, "film_transparent", bool(ft))

    # ---- performance -----------------------------------------------------
    perf = cfg.get("perf")
    if perf:
        cy = get_val(scene, "cycles")
        if perf.get("persistent_data") is not None and cy is not None:
            set_silent(r, "use_persistent_data", bool(perf.get("persistent_data")))
        threads = perf.get("threads")
        if threads is not None:
            if int(threads) <= 0:
                set_silent(r, "threads_mode", "AUTO")
            else:
                set_silent(r, "threads_mode", "FIXED")
                set_silent(r, "threads", int(threads))

    # ---- output behaviour ------------------------------------------------
    out = cfg.get("output")
    if out:
        if out.get("overwrite") is not None:
            set_val(r, "use_overwrite", bool(out.get("overwrite")), "overwrite")
        if out.get("placeholders") is not None:
            set_val(r, "use_placeholder", bool(out.get("placeholders")), "placeholders")
        if out.get("use_stamp") is not None:
            set_silent(r, "use_stamp", bool(out.get("use_stamp")))
        if out.get("use_file_extension") is not None:
            set_silent(r, "use_file_extension", bool(out.get("use_file_extension")))

    # ---- frames ----------------------------------------------------------
    fr = cfg.get("frames")
    if fr:
        step = fr.get("step")
        if step and int(step) > 0:
            set_val(scene, "frame_step", int(step), "frame step")

    # ---- fps, done last so it rescales the final frame range -------------
    f = cfg.get("fps")
    if f and f.get("enabled"):
        old = effective_fps(scene)
        target = float(f.get("fps", old))
        fps_i, fps_b = fps_pair(target)
        mode = (f.get("retime") or "NONE").upper()
        if old <= 0:
            old = 24.0
        k = target / old
        set_silent(r, "fps", fps_i)
        set_silent(r, "fps_base", float(fps_b))
        log("fps %.4f -> %.4f" % (old, effective_fps(scene)))

        if mode != "NONE" and abs(k - 1.0) > 1e-6:
            if mode == "REMAP":
                if not apply_time_remap(scene, old, target):
                    warn("falling back to keyframe rescaling")
                    retime_keyframes(bpy, k)
                    retime_grease_pencil(bpy, k)
            else:
                retime_keyframes(bpy, k)
                retime_grease_pencil(bpy, k)
            retime_caches(bpy, k)
            retime_scene_frames(scene, k)
            if f.get("retime_vse"):
                retime_vse(scene, k)
        elif mode != "NONE":
            log("fps unchanged, nothing to retime")

    log("settings applied")


try:
    main()
except Exception as exc:  # never let this script kill a render
    import traceback
    warn("settings script failed: %s" % exc)
    try:
        traceback.print_exc()
    except Exception:
        pass
