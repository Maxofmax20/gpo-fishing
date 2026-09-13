import { useEffect, useState, useMemo } from "react";
import { Fish, FolderOpen, RefreshCw, Search, Sparkles, Trash2 } from "lucide-react";
import { api } from "../lib/ipc";
import type { CatchRecord } from "../lib/types";
import { Button, Pill, Section, cx } from "../components/primitives";

export default function Journal() {
  const [catches, setCatches] = useState<CatchRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<"all" | "fish" | "fruit">("all");
  const [search, setSearch] = useState("");

  const refresh = async () => {
    setLoading(true);
    try {
      const data = await api.getCatches();
      setCatches(data);
    } catch (e) {
      console.error("Failed to load catches:", e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const clear = async () => {
    if (confirm("Are you sure you want to clear your catch journal history?")) {
      try {
        await api.clearCatches();
        setCatches([]);
      } catch (e) {
        console.error("Failed to clear catches:", e);
      }
    }
  };

  const openCsv = () => {
    api.openCatches().catch((e) => console.error("Failed to open catches:", e));
  };

  const totalFish = useMemo(() => catches.filter((c) => c.kind === "fish").length, [catches]);
  const totalFruits = useMemo(() => catches.filter((c) => c.kind === "fruit").length, [catches]);

  const filtered = useMemo(() => {
    return catches.filter((c) => {
      if (filter === "fish" && c.kind !== "fish") return false;
      if (filter === "fruit" && c.kind !== "fruit") return false;
      if (search.trim()) {
        const q = search.toLowerCase();
        return c.name.toLowerCase().includes(q) || c.raw.toLowerCase().includes(q);
      }
      return true;
    });
  }, [catches, filter, search]);

  return (
    <div className="pb-4 pt-2">
      {/* Top summary stats */}
      <div className="px-4 mb-3 grid grid-cols-3 gap-2">
        <div className="glass rounded-xl p-3 border border-line bg-white/[0.02]">
          <div className="text-[11px] text-fg-mute uppercase tracking-wider font-semibold">Total Catches</div>
          <div className="text-[20px] font-mono font-bold mt-0.5 text-fg">{catches.length}</div>
        </div>
        <div className="glass rounded-xl p-3 border border-line bg-white/[0.02]">
          <div className="text-[11px] text-accent uppercase tracking-wider font-semibold flex items-center gap-1">
            <Fish size={12} /> Fish
          </div>
          <div className="text-[20px] font-mono font-bold mt-0.5 text-accent">{totalFish}</div>
        </div>
        <div className="glass rounded-xl p-3 border border-line bg-white/[0.02]">
          <div className="text-[11px] text-fuchsia-400 uppercase tracking-wider font-semibold flex items-center gap-1">
            <Sparkles size={12} /> Fruits
          </div>
          <div className="text-[20px] font-mono font-bold mt-0.5 text-fuchsia-400">{totalFruits}</div>
        </div>
      </div>

      {/* Action and Filter toolbar */}
      <div className="px-4 mb-3 flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <div className="relative flex-1">
            <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-fg-mute" />
            <input
              type="text"
              placeholder="Search catches..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full pl-8 pr-3 py-1.5 rounded-lg bg-black/30 border border-line text-[12px] placeholder:text-fg-mute focus:outline-none focus:border-accent text-fg"
            />
          </div>
          <Button size="sm" onClick={openCsv} icon={<FolderOpen size={13} />}>
            Open CSV
          </Button>
          <Button size="sm" onClick={refresh} icon={<RefreshCw size={13} className={loading ? "animate-spin" : ""} />} />
          {catches.length > 0 && (
            <Button size="sm" kind="danger" onClick={clear} icon={<Trash2 size={13} />} />
          )}
        </div>

        {/* Filter Pills */}
        <div className="flex items-center gap-1.5">
          <button
            onClick={() => setFilter("all")}
            className={cx(
              "px-3 py-1 rounded-lg text-[12px] font-medium transition-colors",
              filter === "all" ? "bg-accent/20 text-accent border border-accent/30" : "text-fg-dim hover:text-fg hover:bg-white/[0.05]"
            )}
          >
            All ({catches.length})
          </button>
          <button
            onClick={() => setFilter("fish")}
            className={cx(
              "px-3 py-1 rounded-lg text-[12px] font-medium transition-colors",
              filter === "fish" ? "bg-accent/20 text-accent border border-accent/30" : "text-fg-dim hover:text-fg hover:bg-white/[0.05]"
            )}
          >
            Fish ({totalFish})
          </button>
          <button
            onClick={() => setFilter("fruit")}
            className={cx(
              "px-3 py-1 rounded-lg text-[12px] font-medium transition-colors",
              filter === "fruit" ? "bg-fuchsia-500/20 text-fuchsia-400 border border-fuchsia-500/30" : "text-fg-dim hover:text-fg hover:bg-white/[0.05]"
            )}
          >
            Fruits ({totalFruits})
          </button>
        </div>
      </div>

      {/* Catches List */}
      <Section title={`Catches (${filtered.length})`}>
        {filtered.length === 0 ? (
          <div className="p-8 text-center text-fg-mute text-[13px]">
            {loading
              ? "Loading journal entries..."
              : search
              ? "No catches match your search."
              : "No catches recorded yet. Start fishing and catches will appear here automatically."}
          </div>
        ) : (
          <div className="divide-y divide-line max-h-[380px] overflow-y-auto">
            {filtered.map((item, idx) => (
              <div key={idx} className="px-4 py-2.5 flex items-center gap-3 hover:bg-white/[0.02] transition-colors">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-[13px] font-semibold text-fg truncate">{item.name}</span>
                    <Pill tone={item.kind === "fruit" ? "fruit" : "accent"}>{item.kind}</Pill>
                  </div>
                  {item.raw && item.raw !== item.name && (
                    <div className="text-[11px] text-fg-mute truncate font-mono mt-0.5">{item.raw}</div>
                  )}
                </div>
                <div className="text-[11px] font-mono text-fg-dim whitespace-nowrap">{item.timestamp}</div>
              </div>
            ))}
          </div>
        )}
      </Section>
    </div>
  );
}
