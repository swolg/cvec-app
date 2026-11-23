import {render,screen,fireEvent} from "@testing-library/react";
import {it,expect,vi} from "vitest";
import {lazyWorkspace} from "./WorkspaceLoader";
it("reuses a resolved workspace on navigation without another loading state",async()=>{
  const load=vi.fn().mockResolvedValue({default:()=> <p>Workspace ready</p>});
  const Workspace=lazyWorkspace(load,"Review");
  const view=render(<Workspace />);
  await screen.findByText("Workspace ready");
  view.rerender(<p>Other workspace</p>);
  view.rerender(<Workspace />);
  expect(screen.getByText("Workspace ready")).toBeInTheDocument();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  expect(load).toHaveBeenCalledTimes(1);
});
it("retries an unavailable workspace chunk without losing the app shell",async()=>{
  const error=vi.spyOn(console,"error").mockImplementation(()=>undefined);
  const load=vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce({default:()=> <p>Workspace ready</p>});
  const Workspace=lazyWorkspace(load,"Review");
  try{const view=render(<div><p>App shell</p><Workspace /></div>);expect(await screen.findByRole("alert")).toHaveTextContent("Review could not be opened");expect(screen.getByText("App shell")).toBeInTheDocument();fireEvent.click(screen.getByRole("button",{name:"Retry"}));expect(await screen.findByText("Workspace ready")).toBeInTheDocument();view.rerender(<p>Other workspace</p>);view.rerender(<Workspace />);expect(screen.getByText("Workspace ready")).toBeInTheDocument();expect(load).toHaveBeenCalledTimes(2);}finally{error.mockRestore();}
});
