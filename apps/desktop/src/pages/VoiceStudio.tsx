import { useMemo, useState } from "react";
import { api, type Model } from "../api";

interface Preview {
  model: Model;
  url: string;
  note: string;
}

/**
 * Voice Design Studio (§9.3).
 *
 * The workflow of the specification: a natural language description leads to
 * the engine, the model, a generated sample, an audition, an adjusted prompt
 * and finally the saved voice profile. The same description can be rendered
 * by several models so the results can be compared.
 */
export default function VoiceStudio({
  models,
  onSaved,
}: {
  models: Model[];
  onSaved: () => void;
}) {
  const [description, setDescription] = useState(
    "青年女性，稚嫩、害羞、略带怯意，语速偏慢。",
  );
  const [sampleText, setSampleText] = useState("你好，我是你的贴身侍女。");
  const [selected, setSelected] = useState<string[]>([]);
  const [previews, setPreviews] = useState<Preview[]>([]);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [name, setName] = useState("");
  const [language, setLanguage] = useState("Chinese");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  const candidates = useMemo(
    () =>
      models.filter(
        (model) =>
          model.features.includes("voice_design") && model.compatible,
      ),
    [models],
  );

  const toggle = (id: string) =>
    setSelected((current) =>
      current.includes(id)
        ? current.filter((value) => value !== id)
        : [...current, id],
    );

  const generate = async () => {
    if (selected.length === 0) {
      setError("请至少选择一个模型");
      return;
    }
    setBusy(true);
    setError(null);
    setSaved(null);
    const generated: Preview[] = [];
    try {
      for (const id of selected) {
        const model = candidates.find((candidate) => candidate.id === id);
        if (!model) continue;
        const blob = await api.speak({
          model: model.id,
          input: sampleText,
          parameters: {
            family: model.family === "mock" ? "mock" : "qwen3_tts",
            task_type: "VoiceDesign",
            instructions: description,
          },
        });
        generated.push({
          model,
          url: URL.createObjectURL(blob),
          note: notes[model.id] ?? "",
        });
      }
      setPreviews(generated);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const save = async (model: Model) => {
    const voiceName = name.trim() || description.slice(0, 20);
    setBusy(true);
    setError(null);
    try {
      const result = await api.designVoice({
        model: model.id,
        prompt: description,
        name: voiceName,
        language,
        sample_text: sampleText,
      });
      setSaved(`已保存声音「${result.voice.name}」`);
      setName("");
      onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="page">
      <header className="page-header">
        <div>
          <h2>声音设计工作台</h2>
          <p className="muted">
            自然语言描述 → 引擎 → 模型 → 生成样本 → 试听评价 → 调整 Prompt → 保存。
          </p>
        </div>
      </header>

      <div className="studio">
        <div className="studio-input card">
          <label>
            声音描述
            <textarea
              rows={4}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder="例如：青年女性，稚嫩、害羞、略带怯意"
            />
          </label>
          <label>
            试听文本
            <textarea
              rows={2}
              value={sampleText}
              onChange={(event) => setSampleText(event.target.value)}
            />
          </label>
          <div className="model-picker">
            <div className="muted small">选择 Voice Design 模型（可多选对比）</div>
            {candidates.length === 0 && (
              <p className="muted small">
                没有可用于声音设计的模型（可能尚未下载，或本机不兼容）。
              </p>
            )}
            {candidates.map((model) => (
              <label key={model.id} className="checkbox">
                <input
                  type="checkbox"
                  checked={selected.includes(model.id)}
                  onChange={() => toggle(model.id)}
                />
                <span>
                  {model.name}
                  <span className="muted small"> · {model.version}</span>
                </span>
              </label>
            ))}
          </div>
          <button className="primary" disabled={busy} onClick={() => void generate()}>
            {busy ? "生成中…" : "生成样本"}
          </button>
        </div>

        <div className="studio-output">
          {error && <div className="alert error">{error}</div>}
          {saved && <div className="alert ok">{saved}</div>}

          {previews.length === 0 ? (
            <p className="muted">还没有样本。填写描述并选择模型后点击「生成样本」。</p>
          ) : (
            <div className="preview-grid">
              {previews.map((preview) => (
                <article className="card" key={preview.model.id}>
                  <header className="card-header">
                    <h3>{preview.model.name}</h3>
                    <span className="muted small">{preview.model.version}</span>
                  </header>
                  <audio controls src={preview.url} />
                  <label className="small">
                    评价
                    <input
                      value={notes[preview.model.id] ?? ""}
                      placeholder="这个声音如何？"
                      onChange={(event) =>
                        setNotes({
                          ...notes,
                          [preview.model.id]: event.target.value,
                        })
                      }
                    />
                  </label>
                  <div className="field">
                    <input
                      value={name}
                      onChange={(event) => setName(event.target.value)}
                      placeholder="保存为（留空则用描述开头）"
                    />
                    <select
                      value={language}
                      onChange={(event) => setLanguage(event.target.value)}
                    >
                      <option value="Chinese">Chinese</option>
                      <option value="English">English</option>
                      <option value="Japanese">Japanese</option>
                      <option value="auto">自动</option>
                    </select>
                    <button
                      disabled={busy}
                      onClick={() => void save(preview.model)}
                    >
                      保存为声音
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
