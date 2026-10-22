# RMBG Studio (Svelte 5)

Node + Edit mode compositor. Uses the existing FastAPI backend (`python main.py` on :8000).

    npm install
    npm run dev      # http://localhost:5173, proxies /api -> 127.0.0.1:8000

## Adding a node
Edit `src/core/nodes.ts`: add a `NodeDef` (sockets, `fields`, `defaults`, `evaluate`) and put it in `defs`.
Handles, the param form, the thumbnail, the Add button and caching come for free.
Only nodes needing custom UI (like Image) need a Svelte body: register it in `bodies` in `NodeView.svelte`.
