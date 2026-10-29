import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import WidgetPage from "./pages/WidgetPage";
import DetailsPage from "./pages/DetailsPage";
import SettingsPage from "./pages/SettingsPage";
import "./index.css";

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<WidgetPage />} />
        <Route path="/details" element={<DetailsPage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
