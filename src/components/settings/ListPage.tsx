import { LIST_SORTS, type ListSort } from "../../backend";
import { SORT_LABEL } from "../../lib/listView";
import { app, setListSort, setOpenInSearch } from "../../state/app";
import { useStore } from "../../state/store";
import { Select, Switch } from "../ui/Inputs";
import { Group, PageNote, Row } from "./parts";

/** How the vault opens and how the item list is ordered. Both are kept by the core with the other device settings. */
export function ListPage() {
  const openInSearch = useStore(app, (s) => s.device?.openInSearch === true);
  const sort = useStore(app, (s) => s.device?.listSort ?? "updated-desc");
  return (
    <div className="flex flex-col gap-4">
      <PageNote>These apply to this device only. The search box, the order and the filters are also in the list itself.</PageNote>
      <Group title="Vault list" heading={false}>
        <Row title="Open in search" description="After the vault opens, the cursor is already in the search box. On a phone the keyboard comes up with it.">
          <Switch checked={openInSearch} onChange={(on) => void setOpenInSearch(on)} label="Open in search" />
        </Row>
        <Row title="Order" description="How items are sorted when you open the list. You can change it any time from the list.">
          <Select aria-label="Order" value={sort} onChange={(e) => void setListSort(e.target.value as ListSort)} className="w-auto">
            {LIST_SORTS.map((s) => <option key={s} value={s}>{SORT_LABEL[s]}</option>)}
          </Select>
        </Row>
      </Group>
    </div>
  );
}
