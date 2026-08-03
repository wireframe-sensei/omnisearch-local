/** Three staggered bouncing dots, shown wherever a streamed LLM response is in flight. */
export function LoadingDots() {
  return (
    <span className="ml-1 inline-flex items-center gap-0.5 align-middle">
      <span className="size-1 animate-bounce rounded-full bg-current [animation-delay:-0.3s]" />
      <span className="size-1 animate-bounce rounded-full bg-current [animation-delay:-0.15s]" />
      <span className="size-1 animate-bounce rounded-full bg-current" />
    </span>
  );
}
