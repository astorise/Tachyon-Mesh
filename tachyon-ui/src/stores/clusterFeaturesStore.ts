import { createStore } from "zustand/vanilla";
import { resilientInvoke as invoke } from "../utils/network";
import type { ClusterFeatureSet } from "../types/clusterFeatures";
import type { ClusterFeature } from "../registry/ComponentRegistry";

type ClusterFeaturesStore = {
  features: ClusterFeatureSet | null;
  status: "loading" | "ready" | "error";
  fetchFeatures: () => Promise<void>;
};

export const clusterFeaturesStore = createStore<ClusterFeaturesStore>((set) => ({
  features: null,
  status: "loading",
  fetchFeatures: async () => {
    set({ status: "loading" });
    try {
      const features = await invoke<ClusterFeatureSet>("get_cluster_features");
      set({ features, status: "ready" });
    } catch {
      set({ features: null, status: "error" });
    }
  },
}));

window.addEventListener("connection:connected", () => {
  void clusterFeaturesStore.getState().fetchFeatures();
});

export function isFeatureAvailable(feature: ClusterFeature): boolean {
  const { features } = clusterFeaturesStore.getState();
  return features !== null && features[feature] === true;
}
