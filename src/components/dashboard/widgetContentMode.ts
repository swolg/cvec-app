import { useEffect, useRef, useState } from "react";

export type WidgetContentMode = "compact" | "regular" | "expanded";

interface ElementSize {
  width: number;
  height: number;
}

export function useWidgetContentMode<TElement extends HTMLElement>() {
  const ref = useRef<TElement | null>(null);
  const [size, setSize] = useState<ElementSize>({ width: 0, height: 0 });

  useEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === "undefined") {
      return;
    }

    const observer = new ResizeObserver(([entry]) => {
      const { height, width } = entry.contentRect;
      setSize({ height, width });
    });
    observer.observe(element);

    return () => observer.disconnect();
  }, []);

  return [ref, getWidgetContentMode(size), size] as const;
}

function getWidgetContentMode(size: ElementSize): WidgetContentMode {
  const width = size.width || 0;
  const height = size.height || 0;

  if (width > 0 && height > 0 && (width < 560 || height < 230)) {
    return "compact";
  }

  if (width >= 820 && height >= 330) {
    return "expanded";
  }

  return "regular";
}
