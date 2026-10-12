export interface WhiteboardLoadingState {
  id: string;
  kind: "lesson" | "selection";
  title: string;
  detail: string;
}

export function WhiteboardLoadingBlock({
  state,
  left,
  top,
}: {
  state: WhiteboardLoadingState;
  left: number;
  top: number;
}) {
  return (
    <article
      className={`learning-whiteboard-loading-block is-${state.kind}`}
      style={{ left, top }}
      role="status"
      aria-live="polite"
      aria-label={`${state.title}。${state.detail}`}
      data-loading-id={state.id}
    >
      <div className="learning-whiteboard-loading-glow" aria-hidden="true">
        {Array.from({ length: 6 }, (_, index) => (
          <span key={index} />
        ))}
      </div>
      <div className="learning-whiteboard-loading-beam" aria-hidden="true" />
      <div className="learning-whiteboard-loading-copy">
        <span>Octos 正在准备</span>
        <strong>{state.title}</strong>
        <p>{state.detail}</p>
      </div>
      <div className="learning-whiteboard-loading-lines" aria-hidden="true">
        <i />
        <i />
        <i />
      </div>
    </article>
  );
}
