import { Component, lazy, Suspense, useState, type ComponentType, type ReactNode } from "react";
import { Button } from "./ui/button";

class LoadBoundary extends Component<{children: ReactNode; label:string; onRetry:()=>void}, {failed:boolean}> {
  state={failed:false};
  static getDerivedStateFromError(){return {failed:true};}
  render(){
    if(this.state.failed) return <div role="alert" className="grid place-content-center gap-3 p-6"><p>{this.props.label} could not be opened.</p><Button variant="secondary" onClick={this.props.onRetry}>Retry</Button></div>;
    return this.props.children;
  }
}

export function lazyWorkspace<Props extends object>(load:()=>Promise<{default:ComponentType<Props>}>, label:string) {
  // React caches the resolved module on the lazy component itself. Keep that
  // identity across navigation so a warm mount does not suspend again.
  let cachedWorkspace = lazy(load);
  return function DeferredWorkspace(props:Props) {
    const [attempt,setAttempt]=useState(0);
    const [Loaded,setLoaded]=useState(()=>cachedWorkspace);
    function retry() {
      cachedWorkspace = lazy(load);
      setLoaded(()=>cachedWorkspace);
      setAttempt((value)=>value+1);
    }
    return <LoadBoundary key={attempt} label={label} onRetry={retry}><Suspense fallback={<div role="status" className="grid place-content-center p-6 text-sm text-ink-subtle">Opening {label}…</div>}><Loaded {...props} /></Suspense></LoadBoundary>;
  };
}
