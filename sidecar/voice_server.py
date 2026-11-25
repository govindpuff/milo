import argparse
import asyncio
import hashlib
import os
import queue
import sys
import threading
import time
import traceback

import numpy as np

BREEZE_REPO = "mlx-community/Breeze-TTS-2-mlx-4bit"
REF_TEXT = "Oh, um, yeah, I was just thinking about that, actually. It's kind of funny how things work out, you know?"
CHUNK_S = 0.16
CONTEXT_S = 25.0
MAX_FRAMES = 750
TEMPERATURE = 0.9
TOP_P = 1.0
TOP_K = 50


def watch_parent(ppid):
    while True:
        if os.getppid() != ppid:
            os._exit(0)
        time.sleep(1.0)


def pcm16(x):
    return (np.clip(np.asarray(x, dtype=np.float32), -1.0, 1.0) * 32767.0).astype("<i2").tobytes()


class Breeze:
    def __init__(self, model, ref_wav, ref_text):
        import mlx.core as mx

        self.mx = mx
        self.m = model
        self.speaker = model._speaker(None)
        self.decoder = model.audio_tokenizer.decoder
        rate = model.audio_tokenizer.decode_upsample_rate
        self.chunk = max(1, int(CHUNK_S * model.sample_rate / rate))
        self.limit = int(CONTEXT_S * model.sample_rate / rate)
        self.eos = model.backbone_model.embed_tokens(
            mx.full((1, 1, model.num_codebooks), model.config.codebook_eos_token_id, dtype=mx.int32)
        )
        codes = model._encode_reference(ref_wav)
        prefix = [self.text(f"{self.speaker}{ref_text}"), model.backbone_model.embed_tokens(codes), self.eos]
        self.base = model.backbone_model.make_cache()
        model.backbone_model(input_embeddings=mx.concatenate(prefix, axis=1), cache=self.base)
        mx.eval([c.state for c in self.base])
        self.leads = {}
        self.live = None

    def text(self, text):
        m = self.m
        return m.text_encoder_proj(m.text_encoder(m._text_ids(text)[None, :]))

    def fork(self):
        from mlx_audio.lm.models.cache import KVCache

        out = []
        for c in self.base:
            n = KVCache()
            n.state = c.state
            out.append(n)
        return out

    def lead(self, key, text, pcm):
        audio = np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32768.0
        codes = self.m._encode_reference(self.mx.array(audio))
        self.mx.eval(codes)
        self.leads[key] = (text, codes)

    def speak(self, text, instruct, session=None, lead=None):
        mx, m = self.mx, self.m
        target = f"{self.speaker}<ins_bos>{instruct}<ins_eos>{text}" if instruct else f"{self.speaker}{text}"
        live, self.live = self.live, None
        if session is not None and live is not None and live[0] == session and live[2] < self.limit:
            cache, total = live[1], live[2]
            parts = [self.eos, self.text(target)]
        else:
            cache, total = self.fork(), 0
            parts = []
            if lead in self.leads:
                said, codes = self.leads[lead]
                parts += [self.text(f"{self.speaker}{said}"), m.backbone_model.embed_tokens(codes), self.eos]
                total = codes.shape[1]
            parts.append(self.text(target))
            self.decoder.reset_streaming_state()
        hidden = m.backbone_model(input_embeddings=mx.concatenate(parts, axis=1), cache=cache)[:, -1, :]
        pending = []

        def audio(frames):
            codes = mx.array(frames, dtype=mx.int32)[None, :, :]
            out = m._audio_vector(self.decoder.streaming_step(mx.transpose(codes, (0, 2, 1))))
            mx.eval(out)
            return np.asarray(out, dtype=np.float32)

        for _ in range(MAX_FRAMES):
            logits = m._mask_reserved_codec_logits(m.lm_head(hidden))
            first = m._sample(logits, temperature=TEMPERATURE, top_p=TOP_P, top_k=TOP_K, allow_eos=True)
            if first == m.vocab_size:
                break
            frame = m._depth_tokens(
                first,
                hidden,
                unconditional_hidden=None,
                cfg_scale=1.0,
                temperature=TEMPERATURE,
                top_p=TOP_P,
                top_k=TOP_K,
            )
            pending.append(frame)
            total += 1
            if len(pending) >= self.chunk:
                chunk, pending = pending, []
                yield audio(chunk)
            hidden = m.backbone_model(input_ids=mx.array(frame, dtype=mx.int32)[None, None, :], cache=cache)[:, -1, :]
        if pending:
            yield audio(pending)
        if session is not None:
            self.live = (session, cache, total)


def reference(model, voices_dir, design, ref_wav, ref_text):
    import soundfile as sf

    if ref_wav:
        return ref_wav, ref_text
    os.makedirs(voices_dir, exist_ok=True)
    key = hashlib.sha1((BREEZE_REPO + design + REF_TEXT).encode()).hexdigest()[:12]
    path = os.path.join(voices_dir, f"breeze-{key}.wav")
    if not os.path.exists(path):
        print("designing the voice", flush=True)
        parts = [
            np.asarray(r.audio, dtype=np.float32)
            for r in model.generate(text=REF_TEXT, instruct=design, cfg_scale=4.0, split_pattern=None, seed=7)
        ]
        sf.write(path, np.concatenate(parts), model.sample_rate)
    return path, REF_TEXT


def voiced(model, encode, ref_wav, ref_text):
    import mlx.core as mx

    model._encode_reference = encode
    try:
        breeze = Breeze(model, ref_wav, ref_text)
        return breeze.speak, breeze.lead
    except Exception:
        traceback.print_exc()
        print("continuous speech is unavailable, falling back to separate chunks", flush=True)
    codes = encode(ref_wav)
    mx.eval(codes)
    model._encode_reference = lambda _audio: codes

    def speak(text, instruct, session=None, lead=None):
        for r in model.generate(
            text=text,
            ref_audio=ref_wav,
            ref_text=ref_text,
            instruct=instruct,
            stream=True,
            streaming_interval=CHUNK_S,
            split_pattern=None,
        ):
            yield np.asarray(r.audio, dtype=np.float32)

    return speak, None


def named(design, ref_wav, ref_text):
    return f"{ref_wav}|{ref_text}" if ref_wav else design


class Worker(threading.Thread):
    def __init__(self, args):
        super().__init__(daemon=True)
        self.args = args
        self.jobs = queue.Queue()
        self.loaded = threading.Event()
        self.error = None
        self.voice = None

    def use(self, model, encode, design, ref_wav, ref_text):
        speak, lead = voiced(model, encode, *reference(model, self.args.voices_dir, design, ref_wav, ref_text))
        for _ in speak("Hey. Okay, I'm here.", None):
            pass
        self.voice = named(design, ref_wav, ref_text)
        return speak, lead

    def run(self):
        args = self.args
        try:
            from mlx_audio.tts import load

            print(f"loading {BREEZE_REPO}", flush=True)
            model = load(BREEZE_REPO)
            encode = model._encode_reference
            speak, lead = self.use(model, encode, args.design, args.ref_wav, args.ref_text)
        except Exception as e:
            traceback.print_exc()
            self.error = e
            self.loaded.set()
            return
        self.loaded.set()
        while True:
            job = self.jobs.get()
            if job[0] == "lead":
                _, key, text, pcm, done = job
                try:
                    if lead is not None:
                        lead(key, text, pcm)
                except Exception:
                    traceback.print_exc()
                done.set()
                continue
            if job[0] == "design":
                _, design, out = job
                try:
                    out.put(reference(model, args.voices_dir, design, "", "")[0])
                except Exception:
                    traceback.print_exc()
                    out.put(None)
                continue
            if job[0] == "voice":
                _, design, ref_wav, ref_text, out = job
                try:
                    speak, lead = self.use(model, encode, design, ref_wav, ref_text)
                    out.put(True)
                except Exception:
                    traceback.print_exc()
                    out.put(False)
                continue
            _, text, instruct, session, key, out, cancel = job
            try:
                stream = speak(text, instruct, session, key)
                for pcm in stream:
                    if cancel.is_set():
                        break
                    out.put(pcm16(pcm))
                stream.close()
            except Exception:
                traceback.print_exc()
            out.put(None)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--ppid", type=int, required=True)
    ap.add_argument("--voices-dir", required=True)
    ap.add_argument("--design", required=True)
    ap.add_argument("--ref-wav", default="")
    ap.add_argument("--ref-text", default="")
    args = ap.parse_args()

    threading.Thread(target=watch_parent, args=(args.ppid,), daemon=True).start()

    worker = Worker(args)
    worker.start()
    worker.loaded.wait()
    if worker.error is not None:
        return 1
    print("ready", flush=True)

    import uvicorn
    from fastapi import FastAPI, Request
    from fastapi.responses import JSONResponse, StreamingResponse

    app = FastAPI()

    @app.get("/health")
    def health():
        return {"ok": True, "engine": "breeze", "voice": worker.voice}

    @app.post("/design")
    async def design(request: Request):
        body = await request.json()
        out = queue.Queue()
        worker.jobs.put(("design", (body.get("design") or "").strip(), out))
        path = await asyncio.to_thread(out.get)
        if path is None:
            return JSONResponse({"error": "the voice could not be designed"}, status_code=500)
        return {"path": path}

    @app.post("/voice")
    async def voice(request: Request):
        body = await request.json()
        out = queue.Queue()
        worker.jobs.put(("voice", body.get("design") or "", body.get("ref_wav") or "", body.get("ref_text") or "", out))
        if not await asyncio.to_thread(out.get):
            return JSONResponse({"error": "the voice could not be changed"}, status_code=500)
        return {"ok": True}

    @app.post("/speak")
    async def speak(request: Request):
        body = await request.json()
        text = (body.get("text") or "").strip()
        if not text:
            return JSONResponse({"error": "text is required"}, status_code=400)
        instruct = (body.get("instruct") or "").strip() or None
        session, key = body.get("session"), body.get("lead")

        def audio():
            out, cancel = queue.Queue(), threading.Event()
            worker.jobs.put(("speak", text, instruct, session, key, out, cancel))
            try:
                while True:
                    chunk = out.get()
                    if chunk is None:
                        break
                    yield chunk
            finally:
                cancel.set()

        return StreamingResponse(audio(), media_type="audio/pcm")

    @app.post("/lead")
    async def lead(request: Request, id: int, text: str):
        pcm = await request.body()
        done = threading.Event()
        worker.jobs.put(("lead", id, text, pcm, done))
        while not done.is_set():
            await asyncio.sleep(0.01)
        return {"ok": True}

    uvicorn.run(app, host="127.0.0.1", port=args.port, log_level="warning")
    return 0


if __name__ == "__main__":
    sys.exit(main())
