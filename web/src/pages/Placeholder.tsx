export function Placeholder({ title }: { title: string }) {
  return (
    <div className="page" data-ready="1">
      <div className="page-title">
        <h1 className="t-title">{title}</h1>
      </div>
      <p className="t-muted">This page arrives in the next milestone.</p>
    </div>
  );
}
