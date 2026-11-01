import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "./styles/tokens.css";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createHashRouter, RouterProvider } from "react-router";
import { routes } from "./app/routes";

const queryClient = new QueryClient({
  // Local data: no background refetching on focus, and errors surface immediately.
  defaultOptions: { queries: { refetchOnWindowFocus: false, retry: false } },
});
const router = createHashRouter(routes);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
