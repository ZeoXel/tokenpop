#!/usr/bin/env python3
"""Mock model APIs for testing tokenpop's proxy with real agent CLIs.

Serves streamed responses in the wire formats of:
  * OpenAI Responses   POST /v1/responses                       (Codex)
  * Gemini             POST /v1beta/models/<m>:streamGenerateContent?alt=sse  (Gemini CLI)
  * Anthropic Messages POST /v1/messages                        (Claude Code)

Each reply streams a short reasoning passage and a text answer with small
delays, like a real model. Every request is logged to stderr.

usage: mock-upstream.py [PORT]
"""
import json
import sys
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

THINK = "The user wants a short answer. I'll keep it brief and friendly."
ANSWER = ("tokenpop turns every streamed token into a tiny pop, so you can hear "
          "your agent think, write and call tools without watching the screen.")


def words(s):
    out, cur = [], ""
    for ch in s:
        cur += ch
        if ch == " ":
            out.append(cur)
            cur = ""
    return out + ([cur] if cur else [])


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        sys.stderr.write("mock: " + (fmt % args) + "\n")

    def _json(self, code, obj):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def _sse_start(self):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("connection", "close")
        self.end_headers()
        self.close_connection = True

    def _sse(self, data, event=None):
        msg = (f"event: {event}\n" if event else "") + f"data: {json.dumps(data)}\n\n"
        self.wfile.write(msg.encode())
        self.wfile.flush()

    def do_GET(self):
        if self.path.rstrip("/").endswith("/models"):
            return self._json(200, {"object": "list", "data": [{"id": "mock-model", "object": "model"}], "models": []})
        self._json(404, {"error": {"message": "not found"}})

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(n) if n else b""
        try:
            req = json.loads(body or b"{}")
        except ValueError:
            req = {}
        p = self.path
        if p.endswith("/responses"):
            return self.openai_responses(req)
        if ":streamGenerateContent" in p:
            return self.gemini()
        if p.endswith("/messages"):
            return self.anthropic(req)
        self._json(404, {"error": {"message": f"mock has no route for {p}"}})

    # --- OpenAI Responses -------------------------------------------------
    def openai_responses(self, req):
        rid, rs_id, msg_id = "resp_" + uuid.uuid4().hex[:12], "rs_" + uuid.uuid4().hex[:12], "msg_" + uuid.uuid4().hex[:12]
        self._sse_start()
        base = {"id": rid, "object": "response", "status": "in_progress", "model": req.get("model", "mock-model"), "output": []}
        self._sse({"type": "response.created", "response": base}, "response.created")
        self._sse({"type": "response.in_progress", "response": base}, "response.in_progress")
        reasoning = {"type": "reasoning", "id": rs_id, "summary": []}
        self._sse({"type": "response.output_item.added", "output_index": 0, "item": reasoning}, "response.output_item.added")
        self._sse({"type": "response.reasoning_summary_part.added", "item_id": rs_id, "output_index": 0, "summary_index": 0,
                   "part": {"type": "summary_text", "text": ""}}, "response.reasoning_summary_part.added")
        for w in words(THINK):
            time.sleep(0.03)
            self._sse({"type": "response.reasoning_summary_text.delta", "item_id": rs_id, "output_index": 0, "summary_index": 0, "delta": w},
                      "response.reasoning_summary_text.delta")
        self._sse({"type": "response.reasoning_summary_text.done", "item_id": rs_id, "output_index": 0, "summary_index": 0, "text": THINK},
                  "response.reasoning_summary_text.done")
        reasoning_done = {"type": "reasoning", "id": rs_id, "summary": [{"type": "summary_text", "text": THINK}]}
        self._sse({"type": "response.output_item.done", "output_index": 0, "item": reasoning_done}, "response.output_item.done")
        msg = {"type": "message", "id": msg_id, "status": "in_progress", "role": "assistant", "content": []}
        self._sse({"type": "response.output_item.added", "output_index": 1, "item": msg}, "response.output_item.added")
        self._sse({"type": "response.content_part.added", "item_id": msg_id, "output_index": 1, "content_index": 0,
                   "part": {"type": "output_text", "text": "", "annotations": []}}, "response.content_part.added")
        for w in words(ANSWER):
            time.sleep(0.03)
            self._sse({"type": "response.output_text.delta", "item_id": msg_id, "output_index": 1, "content_index": 0, "delta": w},
                      "response.output_text.delta")
        part = {"type": "output_text", "text": ANSWER, "annotations": []}
        self._sse({"type": "response.output_text.done", "item_id": msg_id, "output_index": 1, "content_index": 0, "text": ANSWER},
                  "response.output_text.done")
        self._sse({"type": "response.content_part.done", "item_id": msg_id, "output_index": 1, "content_index": 0, "part": part},
                  "response.content_part.done")
        msg_done = dict(msg, status="completed", content=[part])
        self._sse({"type": "response.output_item.done", "output_index": 1, "item": msg_done}, "response.output_item.done")
        usage = {"input_tokens": 10, "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 40,
                 "output_tokens_details": {"reasoning_tokens": 12}, "total_tokens": 50}
        done = dict(base, status="completed", output=[reasoning_done, msg_done], usage=usage)
        self._sse({"type": "response.completed", "response": done}, "response.completed")

    # --- Gemini -----------------------------------------------------------
    def gemini(self):
        self._sse_start()
        for w in words(THINK):
            time.sleep(0.03)
            self._sse({"candidates": [{"content": {"parts": [{"text": w, "thought": True}], "role": "model"}, "index": 0}]})
        ws = words(ANSWER)
        for i, w in enumerate(ws):
            time.sleep(0.03)
            c = {"content": {"parts": [{"text": w}], "role": "model"}, "index": 0}
            if i == len(ws) - 1:
                c["finishReason"] = "STOP"
            chunk = {"candidates": [c]}
            if i == len(ws) - 1:
                chunk["usageMetadata"] = {"promptTokenCount": 10, "candidatesTokenCount": 40, "totalTokenCount": 50}
            self._sse(chunk)

    # --- Anthropic ---------------------------------------------------------
    def anthropic(self, req):
        if not req.get("stream"):
            return self._json(200, {"id": "msg_mock", "type": "message", "role": "assistant", "model": req.get("model"),
                                    "content": [{"type": "text", "text": ANSWER}], "stop_reason": "end_turn",
                                    "usage": {"input_tokens": 10, "output_tokens": 40}})
        self._sse_start()
        self._sse({"type": "message_start", "message": {"id": "msg_mock", "type": "message", "role": "assistant", "content": [],
                                                         "model": req.get("model"), "usage": {"input_tokens": 10, "output_tokens": 1}}},
                  "message_start")
        self._sse({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}, "content_block_start")
        for w in words(ANSWER):
            time.sleep(0.03)
            self._sse({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": w}}, "content_block_delta")
        self._sse({"type": "content_block_stop", "index": 0}, "content_block_stop")
        self._sse({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 40}}, "message_delta")
        self._sse({"type": "message_stop"}, "message_stop")


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 7318
    sys.stderr.write(f"mock upstream on http://127.0.0.1:{port}\n")
    ThreadingHTTPServer(("127.0.0.1", port), H).serve_forever()
