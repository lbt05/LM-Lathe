/**
 * The voice library and the voice design studio.
 *
 * The voice profiles live on the disk of the local service. This page shows
 * them, plays their samples and creates new ones: either from a written
 * description or by cloning a reference clip.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { api, formatDate, type Model, type Voice } from "../api";
import { useStore } from "../store";
import { Empty, Eyebrow, Icon, Kv, Note, SheetHead } from "../ui";

/** The feature of a model that creates a voice from a description. */
const DESIGN = "voice_design";
/** The feature of a model that creates a voice from a reference clip. */
const CLONING = "voice_cloning";

/** The models that offer the given feature on this machine. */
function candidates(models: Model[], feature: string): Model[] {
  return models.filter(
    (model) => model.features.includes(feature) && model.compatible,
  );
}

/** The models that offer the feature, but not on this machine. */
function rejected(models: Model[], feature: string): Model[] {
  return models.filter(
    (model) => model.features.includes(feature) && !model.compatible,
  );
}

/** A short hint for a voice, e.g. `女声 · 温柔`. */
function summaryOf(voice: Voice): string {
  const parts = [
    voice.gender_presentation,
    voice.age_style,
    voice.personality,
  ].filter((part): part is string => Boolean(part && part.trim()));
  return parts.join(" · ");
}

/** The modal frame of the studio. */
function Modal({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (!dialog.open) dialog.showModal();
    // The escape key closes the studio; the parent decides what happens
    // with the half finished form.
    const onCancel = (event: Event) => {
      event.preventDefault();
      onClose();
    };
    dialog.addEventListener("cancel", onCancel);
    return () => dialog.removeEventListener("cancel", onCancel);
  }, [onClose]);

  return (
    <dialog
      className="dlg dlg--wide"
      ref={ref}
      onClick={(event) => {
        // A click on the backdrop is a click on the dialog itself.
        if (event.target === ref.current) onClose();
      }}
    >
      <div className="dlg__head">
        <span className="dlg__title">{title}</span>
      </div>
      {children}
    </dialog>
  );
}

/** The form of the voice design studio. */
function Studio({
  mode,
  onClose,
}: {
  mode: "design" | "clone";
  onClose: () => void;
}) {
  const store = useStore();
  const feature = mode === "design" ? DESIGN : CLONING;
  const usable = candidates(store.models, feature);
  const blocked = rejected(store.models, feature);

  const [model, setModel] = useState(() => usable[0]?.id ?? "");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [prompt, setPrompt] = useState("");
  const [language, setLanguage] = useState("");
  const [gender, setGender] = useState("");
  const [age, setAge] = useState("");
  const [personality, setPersonality] = useState("");
  const [reference, setReference] = useState("");
  const [referenceText, setReferenceText] = useState("");
  const [sampleText, setSampleText] = useState("");
  const [busy, setBusy] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const selected = usable.find((item) => item.id === model);
  const canSubmit =
    Boolean(model) &&
    Boolean(selected?.installed) &&
    !busy &&
    (mode === "design" ? prompt.trim().length > 0 : reference.trim().length > 0);

  const install = async () => {
    if (!selected) return;
    setInstalling(true);
    try {
      await api.installModel(selected.id);
      store.enqueue({
        id: selected.id,
        name: selected.name,
        percent: 0,
        bytes: 0,
        bytesTotal: 0,
        state: "running",
      });
      store.log("info", `开始下载 ${selected.name}`);
      store.notify(`已加入下载队列：${selected.name}`);
      await store.refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      store.log("bad", `下载 ${selected.name} 失败：${message}`);
      store.notify(message, "bad");
    } finally {
      setInstalling(false);
    }
  };

  const submit = async () => {
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      const body: Record<string, unknown> = {
        model,
        name: name.trim() || undefined,
        language: language.trim() || undefined,
        sample_text: sampleText.trim() || undefined,
      };
      if (mode === "design") {
        body.prompt = prompt.trim();
        body.gender_presentation = gender.trim() || undefined;
        body.age_style = age.trim() || undefined;
        body.personality = personality.trim() || undefined;
        const created = await api.designVoice(body);
        store.log("ok", `已设计声音 ${created.voice.name}`);
        store.notify(`已创建声音 ${created.voice.name}`);
      } else {
        body.reference_audio = reference.trim();
        body.reference_text = referenceText.trim() || undefined;
        body.description = description.trim() || undefined;
        const created = await api.cloneVoice(body);
        store.log("ok", `已克隆声音 ${created.voice.name}`);
        store.notify(`已创建声音 ${created.voice.name}`);
      }
      await store.refresh();
      onClose();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      store.log("bad", `创建声音失败：${message}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={mode === "design" ? "设计声音" : "克隆声音"}
      onClose={onClose}
    >
      <div className="dlg__body">
        {!usable.length ? (
          <Note title="没有可用的模型" tone="warn">
            <p>
              本机上没有支持{mode === "design" ? "声音设计" : "声音克隆"}的模型。
              {blocked.length
                ? `以下模型支持这个功能，但在这台机器上跑不起来：`
                : "请先在模型页安装一个支持该功能的模型。"}
            </p>
            {blocked.length ? (
              <ul>
                {blocked.map((item) => (
                  <li key={item.id}>
                    {item.name} —— {item.compatibility || "与本机不兼容"}
                  </li>
                ))}
              </ul>
            ) : null}
          </Note>
        ) : null}

        {usable.length ? (
          <>
            <label className="field">
              <span className="field__label">模型</span>
              <select
                className="input"
                value={model}
                onChange={(event) => setModel(event.target.value)}
              >
                {usable.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.name}
                    {item.installed ? "" : "（未安装）"}
                  </option>
                ))}
              </select>
              <span className="field__hint">
                {selected?.installed
                  ? "这个模型已经装好，可以直接使用。"
                  : "这个模型还没有下载，先装好它才能生成声音。"}
              </span>
            </label>

            {mode === "design" ? (
              <label className="field">
                <span className="field__label">声音描述</span>
                <textarea
                  className="input input--area"
                  rows={4}
                  value={prompt}
                  placeholder="青年女性，稚嫩、害羞、略带怯意。"
                  onChange={(event) => setPrompt(event.target.value)}
                />
                <span className="field__hint">
                  用自然语言描述这个声音，模型会照着描述合成试听样本。
                </span>
              </label>
            ) : (
              <>
                <label className="field">
                  <span className="field__label">参考音频</span>
                  <input
                    className="input"
                    value={reference}
                    placeholder="/Users/me/reference.wav"
                    onChange={(event) => setReference(event.target.value)}
                  />
                  <span className="field__hint">
                    填写本机音频文件的绝对路径，引擎会读取它来复制音色。
                  </span>
                </label>
                <label className="field">
                  <span className="field__label">参考文本</span>
                  <input
                    className="input"
                    value={referenceText}
                    placeholder="参考音频里说的内容"
                    onChange={(event) => setReferenceText(event.target.value)}
                  />
                  <span className="field__hint">
                    给出的转写文本越准，克隆出来的音色越稳。
                  </span>
                </label>
              </>
            )}

            <div className="fieldgrid">
              <label className="field">
                <span className="field__label">名称</span>
                <input
                  className="input"
                  value={name}
                  placeholder="小艾"
                  onChange={(event) => setName(event.target.value)}
                />
              </label>
              <label className="field">
                <span className="field__label">语言</span>
                <input
                  className="input"
                  value={language}
                  placeholder="zh"
                  onChange={(event) => setLanguage(event.target.value)}
                />
              </label>
            </div>

            {mode === "design" ? (
              <div className="fieldgrid">
                <label className="field">
                  <span className="field__label">性别</span>
                  <input
                    className="input"
                    value={gender}
                    placeholder="女声"
                    onChange={(event) => setGender(event.target.value)}
                  />
                </label>
                <label className="field">
                  <span className="field__label">年龄</span>
                  <input
                    className="input"
                    value={age}
                    placeholder="青年"
                    onChange={(event) => setAge(event.target.value)}
                  />
                </label>
                <label className="field">
                  <span className="field__label">性格</span>
                  <input
                    className="input"
                    value={personality}
                    placeholder="害羞"
                    onChange={(event) => setPersonality(event.target.value)}
                  />
                </label>
              </div>
            ) : (
              <label className="field">
                <span className="field__label">描述</span>
                <input
                  className="input"
                  value={description}
                  placeholder="从录音复制的播音音色"
                  onChange={(event) => setDescription(event.target.value)}
                />
              </label>
            )}

            <label className="field">
              <span className="field__label">试听文本</span>
              <input
                className="input"
                value={sampleText}
                placeholder="你好，这是这个声音的试听样本。"
                onChange={(event) => setSampleText(event.target.value)}
              />
              <span className="field__hint">
                留空时使用默认文本，生成的样本会存进声音档案。
              </span>
            </label>

            {error ? (
              <Note title="创建失败" tone="bad">
                <p>{error}</p>
              </Note>
            ) : null}
          </>
        ) : null}
      </div>
      <div className="dlg__foot">
        <button className="btn" type="button" onClick={onClose}>
          取消
        </button>
        {selected && !selected.installed ? (
          <button
            className="btn btn--primary"
            type="button"
            disabled={installing}
            onClick={() => void install()}
          >
            <Icon name="download" />
            {installing ? "已加入队列…" : "下载模型"}
          </button>
        ) : (
          <button
            className="btn btn--primary"
            type="button"
            disabled={!canSubmit}
            onClick={() => void submit()}
          >
            <Icon name={mode === "design" ? "bulb" : "copy"} />
            {busy ? "生成中…" : mode === "design" ? "生成声音" : "克隆声音"}
          </button>
        )}
      </div>
    </Modal>
  );
}

/** The player of one stored sample. */
function Sample({ voice, sample }: { voice: Voice; sample: string }) {
  return (
    <div className="sample">
      <span className="sample__name" title={sample}>
        {sample}
      </span>
      <audio
        className="sample__player"
        controls
        preload="none"
        src={api.sampleUrl(voice.id, sample)}
      />
    </div>
  );
}

/** The library of the stored voices. */
export default function Voices() {
  const store = useStore();
  const [query, setQuery] = useState("");
  const [modelFilter, setModelFilter] = useState("");
  const [languageFilter, setLanguageFilter] = useState("");
  const [studio, setStudio] = useState<"design" | "clone" | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const voices = store.voices;

  const languages = useMemo(() => {
    const found = new Set<string>();
    for (const voice of voices) {
      if (voice.language) found.add(voice.language);
    }
    return [...found].sort();
  }, [voices]);

  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return voices.filter((voice) => {
      if (modelFilter && voice.model_id !== modelFilter) return false;
      if (languageFilter && voice.language !== languageFilter) return false;
      if (!needle) return true;
      return [
        voice.name,
        voice.description,
        voice.model_id,
        voice.engine,
        voice.language ?? "",
        voice.personality ?? "",
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
  }, [voices, query, modelFilter, languageFilter]);

  const remove = async (voice: Voice) => {
    setConfirmDelete(null);
    try {
      await api.deleteVoice(voice.id);
      store.log("warn", `已删除声音 ${voice.name}`);
      store.notify(`已删除 ${voice.name}`);
      if (store.inspector?.kind === "voice" && store.inspector.id === voice.id) {
        store.setInspector(null);
      }
      await store.refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      store.log("bad", `删除声音失败：${message}`);
      store.notify(message, "bad");
    }
  };

  const open = (voice: Voice) =>
    store.setInspector({ kind: "voice", id: voice.id, tab: "overview" });

  return (
    <>
      <section className="sheet">
        <SheetHead
          eyebrow="VOICE LIBRARY"
          title="声音库"
          meta={`${voices.length} 个声音`}
          tools={
            <>
              <button
                className="btn btn--primary"
                type="button"
                disabled={!store.base}
                onClick={() => setStudio("design")}
              >
                <Icon name="bulb" />
                设计声音
              </button>
              <button
                className="btn"
                type="button"
                disabled={!store.base}
                onClick={() => setStudio("clone")}
              >
                <Icon name="copy" />
                克隆声音
              </button>
            </>
          }
        />

        <div className="filters">
          <input
            className="finput"
            type="search"
            value={query}
            placeholder="搜索声音…"
            aria-label="搜索声音"
            onChange={(event) => setQuery(event.target.value)}
          />
          <span className="fgroup">
            <span className="fgroup__label">模型</span>
            <button
              className="ftog"
              type="button"
              aria-pressed={!modelFilter}
              onClick={() => setModelFilter("")}
            >
              全部
            </button>
            {[...new Set(voices.map((voice) => voice.model_id))]
              .sort()
              .map((id) => (
                <button
                  className="ftog"
                  type="button"
                  key={id}
                  aria-pressed={modelFilter === id}
                  onClick={() => setModelFilter(id)}
                >
                  {id}
                </button>
              ))}
          </span>
          <span className="fgroup">
            <span className="fgroup__label">语言</span>
            <button
              className="ftog"
              type="button"
              aria-pressed={!languageFilter}
              onClick={() => setLanguageFilter("")}
            >
              全部
            </button>
            {languages.map((language) => (
              <button
                className="ftog"
                type="button"
                key={language}
                aria-pressed={languageFilter === language}
                onClick={() => setLanguageFilter(language)}
              >
                {language}
              </button>
            ))}
          </span>
        </div>

        {shown.length ? (
          <div className="tablewrap">
            <table className="tbl">
              <thead>
                <tr>
                  <th>名称</th>
                  <th>描述</th>
                  <th>模型</th>
                  <th>引擎</th>
                  <th>语言</th>
                  <th className="num">样本</th>
                  <th className="num">创建时间</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {shown.map((voice) => (
                  <tr
                    key={voice.id}
                    data-act
                    tabIndex={0}
                    aria-selected={
                      store.inspector?.kind === "voice" &&
                      store.inspector.id === voice.id
                    }
                    onClick={() => open(voice)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        open(voice);
                      }
                    }}
                  >
                    <td>
                      <span className="cellstack">
                        <span className="name">{voice.name}</span>
                        <span className="sub">{voice.id}</span>
                      </span>
                    </td>
                    <td>{voice.description || "—"}</td>
                    <td className="mono">{voice.model_id}</td>
                    <td className="mono">{voice.engine}</td>
                    <td>{voice.language || "—"}</td>
                    <td className="num">{voice.samples.length}</td>
                    <td className="num">{formatDate(voice.created_at)}</td>
                    <td>
                      <span className="tbl__actions">
                        <button
                          className="btn btn--icon"
                          type="button"
                          title="删除"
                          aria-label={`删除 ${voice.name}`}
                          onClick={(event) => {
                            event.stopPropagation();
                            setConfirmDelete(voice.id);
                          }}
                        >
                          <Icon name="trash" />
                        </button>
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <Empty
            title={voices.length ? "没有匹配的声音" : "声音库还是空的"}
            text={
              voices.length
                ? "换一个关键词或筛选条件试试。"
                : "声音档案保存在本机，与推理引擎分开存放。可以先用描述设计一个声音，或用一段录音克隆一个。"
            }
            action={
              !voices.length ? (
                <button
                  className="btn btn--primary"
                  type="button"
                  disabled={!store.base}
                  onClick={() => setStudio("design")}
                >
                  <Icon name="bulb" />
                  设计第一个声音
                </button>
              ) : null
            }
          />
        )}
      </section>

      {studio ? (
        <Studio mode={studio} onClose={() => setStudio(null)} />
      ) : null}

      {confirmDelete ? (
        <Modal title="删除声音" onClose={() => setConfirmDelete(null)}>
          <div className="dlg__body">
            <Note title="这个动作不能撤销" tone="warn">
              <p>
                声音档案
                {voices.find((voice) => voice.id === confirmDelete)?.name
                  ? `「${voices.find((voice) => voice.id === confirmDelete)?.name}」`
                  : ""}
                连同它的试听样本会从磁盘上删除。
              </p>
            </Note>
          </div>
          <div className="dlg__foot">
            <button
              className="btn"
              type="button"
              onClick={() => setConfirmDelete(null)}
            >
              取消
            </button>
            <button
              className="btn btn--danger"
              type="button"
              onClick={() => {
                const voice = voices.find((item) => item.id === confirmDelete);
                if (voice) void remove(voice);
              }}
            >
              <Icon name="trash" />
              删除
            </button>
          </div>
        </Modal>
      ) : null}
    </>
  );
}

/** The detail panel of one voice. */
export function VoiceInspector({
  id,
  tab,
}: {
  id: string;
  tab: string;
}) {
  const store = useStore();
  const voice = store.voices.find((item) => item.id === id);
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(voice?.name ?? "");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setName(voice?.name ?? "");
    setRenaming(false);
  }, [voice?.name]);

  const save = async () => {
    if (!voice) return;
    setBusy(true);
    try {
      await api.updateVoice(voice.id, { name: name.trim() || voice.name });
      store.log("ok", `已重命名声音 ${name.trim() || voice.name}`);
      store.notify("已保存");
      setRenaming(false);
      await store.refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      store.log("bad", `重命名失败：${message}`);
      store.notify(message, "bad");
    } finally {
      setBusy(false);
    }
  };

  if (!voice) {
    return (
      <>
        <div className="insp__head">
          <span className="cellstack">
            <Eyebrow>VOICE</Eyebrow>
            <span className="insp__title">声音不存在</span>
            <span className="insp__sub">{id}</span>
          </span>
          <button
            className="btn btn--icon insp__x"
            type="button"
            aria-label="关闭"
            onClick={() => store.setInspector(null)}
          >
            <Icon name="x" />
          </button>
        </div>
      </>
    );
  }

  const summary = summaryOf(voice);

  return (
    <>
      <div className="insp__head">
        <span className="cellstack">
          <Eyebrow>VOICE</Eyebrow>
          <span className="insp__title">{voice.name}</span>
          <span className="insp__sub">{voice.id}</span>
        </span>
        <button
          className="btn btn--icon insp__x"
          type="button"
          aria-label="关闭"
          onClick={() => store.setInspector(null)}
        >
          <Icon name="x" />
        </button>
      </div>

      <div className="insp__tabs">
        <button
          type="button"
          aria-selected={tab === "overview"}
          onClick={() =>
            store.setInspector({ kind: "voice", id: voice.id, tab: "overview" })
          }
        >
          概览
        </button>
        <button
          type="button"
          aria-selected={tab === "samples"}
          onClick={() =>
            store.setInspector({ kind: "voice", id: voice.id, tab: "samples" })
          }
        >
          样本
        </button>
      </div>

      <div className="insp__body">
        {tab === "samples" ? (
          voice.samples.length ? (
            voice.samples.map((sample) => (
              <Sample key={sample} voice={voice} sample={sample} />
            ))
          ) : (
            <Empty
              title="还没有样本"
              text="用这个声音合成一次语音，样本就会出现在这里。"
            />
          )
        ) : (
          <>
            {voice.description ? (
              <Note title="描述">{voice.description}</Note>
            ) : null}

            <div className="kv">
              <Kv k="模型" v={voice.model_id} mono />
              <Kv k="引擎" v={voice.engine} mono />
              <Kv k="语言" v={voice.language || "—"} />
              {voice.gender_presentation ? (
                <Kv k="性别" v={voice.gender_presentation} />
              ) : null}
              {voice.age_style ? <Kv k="年龄" v={voice.age_style} /> : null}
              {voice.personality ? (
                <Kv k="性格" v={voice.personality} />
              ) : null}
              <Kv k="样本" v={`${voice.samples.length} 个`} />
              <Kv k="创建" v={formatDate(voice.created_at)} mono />
              <Kv k="更新" v={formatDate(voice.updated_at)} mono />
            </div>

            {summary ? (
              <Note title="特征">{summary}</Note>
            ) : null}

            {voice.samples.length ? (
              <div className="samples">
                <Eyebrow>试听</Eyebrow>
                {voice.samples.map((sample) => (
                  <Sample key={sample} voice={voice} sample={sample} />
                ))}
              </div>
            ) : null}
          </>
        )}
      </div>

      <div className="insp__foot">
        {renaming ? (
          <>
            <input
              className="input"
              value={name}
              aria-label="声音名称"
              onChange={(event) => setName(event.target.value)}
            />
            <button
              className="btn btn--primary"
              type="button"
              disabled={busy || !name.trim()}
              onClick={() => void save()}
            >
              保存
            </button>
            <button
              className="btn"
              type="button"
              onClick={() => {
                setName(voice.name);
                setRenaming(false);
              }}
            >
              取消
            </button>
          </>
        ) : (
          <>
            <button
              className="btn"
              type="button"
              onClick={() => setRenaming(true)}
            >
              <Icon name="more" />
              重命名
            </button>
            <a
              className="btn"
              href={`#/services`}
              onClick={(event) => {
                event.preventDefault();
                store.setView("services");
              }}
            >
              <Icon name="play" />
              去合成
            </a>
          </>
        )}
      </div>
    </>
  );
}
