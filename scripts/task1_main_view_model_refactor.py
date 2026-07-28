from __future__ import annotations

import re
import urllib.request
from dataclasses import dataclass
from pathlib import Path

SOURCE_SHA = "486f2df79ed407620771c08033c6e2002b557548"
BASE_URL = f"https://raw.githubusercontent.com/ekkus93/silent_disco/{SOURCE_SHA}"
DIRECTORY = Path("app/src/main/java/com/ekkus/silentdisco/app")
SOURCE_FILES = [
    "MainViewModel.kt",
    "MainViewModelAudioPipeline.kt",
    "MainViewModelDemo.kt",
    "MainViewModelDiagnostics.kt",
    "MainViewModelHostPlayback.kt",
    "MainViewModelListenerPlayback.kt",
    "MainViewModelPersistence.kt",
    "MainViewModelSupport.kt",
    "MainViewModelSynchronization.kt",
    "MainViewModelTransport.kt",
]


def download_sources() -> None:
    DIRECTORY.mkdir(parents=True, exist_ok=True)
    for filename in SOURCE_FILES:
        relative = f"app/src/main/java/com/ekkus/silentdisco/app/{filename}"
        with urllib.request.urlopen(f"{BASE_URL}/{relative}") as response:
            (DIRECTORY / filename).write_bytes(response.read())


def mask(text: str) -> str:
    result = list(text)
    state = "code"
    block_depth = 0
    index = 0
    while index < len(text):
        def blank(position: int) -> None:
            if text[position] != "\n":
                result[position] = " "

        if state == "line":
            blank(index)
            if text[index] == "\n":
                state = "code"
            index += 1
            continue
        if state == "block":
            if text.startswith("/*", index):
                blank(index)
                blank(index + 1)
                block_depth += 1
                index += 2
            elif text.startswith("*/", index):
                blank(index)
                blank(index + 1)
                block_depth -= 1
                index += 2
                if block_depth == 0:
                    state = "code"
            else:
                blank(index)
                index += 1
            continue
        if state in {"string", "char"}:
            quote = '"' if state == "string" else "'"
            blank(index)
            if text[index] == "\\" and index + 1 < len(text):
                blank(index + 1)
                index += 2
            elif text[index] == quote:
                state = "code"
                index += 1
            else:
                index += 1
            continue
        if state == "triple":
            if text.startswith('"""', index):
                blank(index)
                blank(index + 1)
                blank(index + 2)
                index += 3
                state = "code"
            else:
                blank(index)
                index += 1
            continue

        if text.startswith("//", index):
            blank(index)
            blank(index + 1)
            state = "line"
            index += 2
        elif text.startswith("/*", index):
            blank(index)
            blank(index + 1)
            state = "block"
            block_depth = 1
            index += 2
        elif text.startswith('"""', index):
            blank(index)
            blank(index + 1)
            blank(index + 2)
            state = "triple"
            index += 3
        elif text[index] == '"':
            blank(index)
            state = "string"
            index += 1
        elif text[index] == "'":
            blank(index)
            state = "char"
            index += 1
        else:
            index += 1

    if state not in {"code", "line"}:
        raise RuntimeError(f"unterminated Kotlin lexical state: {state}")
    return "".join(result)


def close_brace(text: str, opening: int) -> int:
    depth = 0
    for index in range(opening, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return index
    raise RuntimeError("unmatched opening brace")


@dataclass(frozen=True)
class Move:
    declaration: str
    wrapper: str | None
    implementation: str
    destination: str


MOVES = [
    Move("    fun createHostSession(): Boolean {", "    fun createHostSession(): Boolean = createHostSessionImpl()\n", "internal fun MainViewModel.createHostSessionImpl(): Boolean {", "MainViewModelHostActions.kt"),
    Move("    fun approveJoinRequest(request: JoinRequest) {", "    fun approveJoinRequest(request: JoinRequest) = approveJoinRequestImpl(request)\n", "internal fun MainViewModel.approveJoinRequestImpl(request: JoinRequest) {", "MainViewModelHostActions.kt"),
    Move("    fun rejectJoinRequest(request: JoinRequest) {", "    fun rejectJoinRequest(request: JoinRequest) = rejectJoinRequestImpl(request)\n", "internal fun MainViewModel.rejectJoinRequestImpl(request: JoinRequest) {", "MainViewModelHostActions.kt"),
    Move("    fun startHostPlayback() {", "    fun startHostPlayback() = startHostPlaybackImpl()\n", "internal fun MainViewModel.startHostPlaybackImpl() {", "MainViewModelHostActions.kt"),
    Move("    fun pauseHostPlayback() {", "    fun pauseHostPlayback() = pauseHostPlaybackImpl()\n", "internal fun MainViewModel.pauseHostPlaybackImpl() {", "MainViewModelHostActions.kt"),
    Move("    fun stopHostPlayback() {", "    fun stopHostPlayback() = stopHostPlaybackImpl()\n", "internal fun MainViewModel.stopHostPlaybackImpl() {", "MainViewModelHostActions.kt"),
    Move("    fun endSession() {", "    fun endSession() = endSessionImpl()\n", "internal fun MainViewModel.endSessionImpl() {", "MainViewModelHostActions.kt"),
    Move("    fun scanForSessions() {", "    fun scanForSessions() = scanForSessionsImpl()\n", "internal fun MainViewModel.scanForSessionsImpl() {", "MainViewModelListenerActions.kt"),
    Move("    fun requestJoin() {", "    fun requestJoin() = requestJoinImpl()\n", "internal fun MainViewModel.requestJoinImpl() {", "MainViewModelListenerActions.kt"),
    Move("    internal fun startTransportListenerPlayback(sessionId: SessionId, streamId: StreamId, format: AudioFormatSpec = AudioFormatSpec()) {", None, "internal fun MainViewModel.startTransportListenerPlayback(sessionId: SessionId, streamId: StreamId, format: AudioFormatSpec = AudioFormatSpec()) {", "MainViewModelListenerActions.kt"),
]


def refactor() -> None:
    path = DIRECTORY / "MainViewModel.kt"
    source = path.read_text(encoding="utf-8")
    prefix = source[: source.index("class MainViewModel")]
    generated: dict[str, list[str]] = {}

    for move in MOVES:
        code = mask(source)
        start = code.find(move.declaration)
        if start < 0:
            raise RuntimeError(f"missing declaration: {move.declaration}")
        opening = code.find("{", start)
        end = close_brace(code, opening) + 1
        while end < len(source) and source[end] in " \t":
            end += 1
        if end < len(source) and source[end] == "\n":
            end += 1
        block = "\n".join(
            line[4:] if line.startswith("    ") else line
            for line in source[start:end].rstrip().splitlines()
        )
        original = move.declaration[4:]
        if block.count(original) != 1:
            raise RuntimeError(f"unexpected declaration count: {original}")
        generated.setdefault(move.destination, []).append(
            block.replace(original, move.implementation, 1)
        )
        source = source[:start] + (move.wrapper or "") + source[end:]

    path.write_text(re.sub(r"\n{3,}", "\n\n", source).rstrip() + "\n", encoding="utf-8")
    for filename, blocks in generated.items():
        (DIRECTORY / filename).write_text(
            prefix.rstrip() + "\n\n" + "\n\n".join(blocks) + "\n",
            encoding="utf-8",
        )


def verify() -> None:
    files = sorted(DIRECTORY.glob("MainViewModel*.kt"))
    if not files:
        raise RuntimeError("no MainViewModel source files found")
    for path in files:
        lines = len(path.read_text(encoding="utf-8").splitlines())
        print(f"{lines:4d} {path.as_posix()}")
        if lines >= 800:
            raise RuntimeError(f"{path} has {lines} lines")


def main() -> None:
    download_sources()
    refactor()
    verify()


if __name__ == "__main__":
    main()
