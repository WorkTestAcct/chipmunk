import { Component, OnDestroy, ChangeDetectorRef, AfterContentInit, ViewChild, AfterViewInit, Input, ElementRef } from '@angular/core';
import * as Toolkit from 'chipmunk.client.toolkit';
import { Subscription, Subject } from 'rxjs';
import SessionsService from '../../../../services/service.sessions.tabs';
import SidebarSessionsService from '../../../../services/service.sessions.sidebar';
import SearchSessionsService, { IRequest, IChartRequest } from '../../../../services/service.sessions.search';
import { IRequestItem } from './request/component';
import { IChartItem } from './chartentry/component';
import { IRequestItem as IRequestDetailsItem } from './detailsrequest/component';
import { IChartItem as IChartDetailsItem, IOnChangeEvent } from './detailschart/component';
import { NotificationsService } from '../../../../services.injectable/injectable.service.notifications';
import { SidebarAppSearchManagerControlsComponent } from './controls/component';
import { ControllerSessionTab } from '../../../../controller/controller.session.tab';
import ContextMenuService, { IMenu, IMenuItem } from '../../../../services/standalone/service.contextmenu';
import ToolbarSessionsService from '../../../../services/service.sessions.toolbar';
import LayoutStateService from '../../../../services/standalone/service.layout.state';

interface IState {
    _ng_selectedEntryIndex: number;
    _ng_filename: string | undefined;
    _filename: string | undefined;
    _changed: boolean;
}

enum EEntityType {
    filters = 'filters',
    charts = 'charts',
}

@Component({
    selector: 'app-sidebar-app-search-requests',
    templateUrl: './template.html',
    styleUrls: ['./styles.less']
})

export class SidebarAppSearchRequestsComponent implements OnDestroy, AfterContentInit, AfterViewInit {

    public static StateKey = 'sidebar-app-search-requests';

    @ViewChild('listrequests', {static: false}) _listRequestsElmRef: ElementRef;
    @ViewChild('listcharts', {static: false}) _listChartsElmRef: ElementRef;

    public _ng_entries: Array<IRequestItem | IChartItem> = [];
    public _ng_selectedEntry: IRequestDetailsItem | IChartDetailsItem | undefined;
    public _ng_selectedEntryIndex: number = -1;
    public _ng_filename: string | undefined;

    private _subscriptions: { [key: string]: Subscription | undefined } = { };
    private _logger: Toolkit.Logger = new Toolkit.Logger('SidebarAppSearchRequestsComponent');
    private _destroyed: boolean = false;
    private _session: ControllerSessionTab | undefined;
    private _focused: boolean = false;
    private _filename: string | undefined;
    private _changed: boolean = false;
    private _subjectsRequests: {
        onEdit: Subject<IRequest>,
        onEditCancel: Subject<void>,
        onFileReset: Subject<void>,
        onChanges: Subject<void>,
    } = {
        onEdit: new Subject<IRequest>(),
        onEditCancel: new Subject<void>(),
        onFileReset: new Subject<void>(),
        onChanges: new Subject<void>(),
    };
    private _subjectsCharts: {
        onEdit: Subject<IChartRequest>,
        onEditCancel: Subject<void>,
        onFileReset: Subject<void>,
        onChanges: Subject<void>,
    } = {
        onEdit: new Subject<IChartRequest>(),
        onEditCancel: new Subject<void>(),
        onFileReset: new Subject<void>(),
        onChanges: new Subject<void>(),
    };

    constructor(private _cdRef: ChangeDetectorRef,
                private _notifications: NotificationsService) {
        this._subscriptions.onRequestsUpdated = SearchSessionsService.getObservable().onRequestsUpdated.subscribe(this._onEntriesUpdated.bind(this, EEntityType.filters));
        this._subscriptions.onChartsUpdated = SearchSessionsService.getObservable().onChartsUpdated.subscribe(this._onEntriesUpdated.bind(this, EEntityType.charts));
        this._subscriptions.onSessionChange = SessionsService.getObservable().onSessionChange.subscribe(this._onSessionChange.bind(this));
        this._onKeyPress = this._onKeyPress.bind(this);
        this._session = SessionsService.getActive();
        this._ng_entries = [
            ...this._getRequestItems(this._session.getSessionSearch().getFiltersAPI().getStored()),
            ...this._getChartsItems(this._session.getSessionSearch().getChartsAPI().getStored())
        ];
    }

    public ngOnDestroy() {
        this._destroyed = true;
        Object.keys(this._subscriptions).forEach((key: string) => {
            this._subscriptions[key].unsubscribe();
        });
        window.removeEventListener('keyup', this._onKeyPress);
    }

    public ngAfterContentInit() {
        const session: ControllerSessionTab = SessionsService.getActive();
        if (session === undefined) {
            return;
        }
        this._subscriptions.onSearchProcessing = session.getSessionSearch().getFiltersAPI().getObservable().onSearchProcessing.subscribe(this._onSearchProcessing.bind(this));
    }

    public ngAfterViewInit() {
        this._loadState();
        window.addEventListener('keyup', this._onKeyPress);
        SidebarSessionsService.setTitleInjection({
            factory: SidebarAppSearchManagerControlsComponent,
            resolved: false,
            inputs: {
                setCurrentFilename: this._setCurrentFileName.bind(this),
                onFileReset: this._subjectsRequests.onFileReset.asObservable(),
                onChanges: this._subjectsRequests.onChanges.asObservable(),
                dropChangesFlag: () => {
                    this._changed = false;
                    this._saveState();
                },
                filename: this._filename,
                changed: this._changed,
            }
        });
    }

    public _ng_onRemoveAll() {
        this._session.getSessionSearch().getFiltersAPI().removeAllStored();
        this._session.getSessionSearch().getChartsAPI().removeAllStored();
        this._subjectsRequests.onFileReset.next();
        this._ng_filename = undefined;
        this._filename = undefined;
        this._forceUpdate();
    }

    public _ng_onListFocus() {
        this._focused = true;
    }

    public _ng_onListBlur() {
        this._focused = undefined;
    }

    public _ng_onContexMenu(event: MouseEvent, request: IRequestItem | IChartItem, type: EEntityType) {
        if (request === undefined) {
            return;
        }
        const items: IMenuItem[] = [];
        items.push(...[
            {
                caption: `Edit`,
                handler: () => {
                    if (type === EEntityType.filters) {
                        this._subjectsRequests.onEdit.next((request as IRequestItem).request);
                    } else {
                        this._subjectsCharts.onEdit.next((request as IChartItem).request);
                    }
                },
            },
            { /* delimiter */ },
            {
                caption: `Show matches`,
                handler: () => {
                    this._requestSearch(request);
                },
            },
            { /* delimiter */ },
            {
                caption: request.request.active ? `Deactivate` : `Activate`,
                handler: () => {
                    if (type === EEntityType.filters) {
                        this._onChangeStateRequest((request as IRequestItem).request, !request.request.active);
                    } else {
                        this._onChangeStateChart((request as IChartItem).request, !request.request.active);
                    }
                },
            },
            { /* delimiter */ },
            {
                caption: `Deactivate all`,
                handler: () => {
                    this._toggleAllExcept(undefined, true);
                },
            },
            {
                caption: `Activate all`,
                handler: () => {
                    this._toggleAllExcept(undefined, false);
                },
            },
            {
                caption: `Deactivate all except this`,
                handler: () => {
                    this._toggleAllExcept(request.request.reg.source, true);
                },
            },
            {
                caption: `Activate all except this`,
                handler: () => {
                    this._toggleAllExcept(request.request.reg.source, false);
                },
            },
            { /* delimiter */ },
            {
                caption: `Remove`,
                handler: () => {
                    if (type === EEntityType.filters) {
                        this._onRemoveRequest((request as IRequestItem).request);
                    } else {
                        this._onRemoveChart((request as IChartItem).request);
                    }
                },
            },
            {
                caption: `Remove All`,
                handler: () => {
                    this._ng_onRemoveAll();
                },
            },
            { /* delimiter */ },
            {
                caption: `Convert to ${type === EEntityType.filters ? 'chart' : 'filter'}`,
                handler: () => {
                    this._convert(request, type);
                },
            },
        ]);
        ContextMenuService.show({
            items: items,
            x: event.pageX,
            y: event.pageY,
        });
    }

    public _ng_getEntryType(entry: IRequestItem | IChartItem): EEntityType {
        if ((entry.request as any).type !== undefined) {
            // Field "type" has only chart
            return EEntityType.charts;
        } else {
            return EEntityType.filters;
        }
    }

    public _ng_onDblClick(request: IRequestItem | IChartItem) {
        this._requestSearch(request);
    }

    private _requestSearch(request: IRequestItem | IChartItem) {
        if (this._session === undefined) {
            return;
        }
        // Open toolbar
        LayoutStateService.toolbarMax();
        // Open sidebar
        ToolbarSessionsService.setActive(ToolbarSessionsService.getDefaultsGuids().search);
        // Trigger search request
        this._session.getSessionSearch().getFiltersAPI().requestSearch(request.request.reg.source);
    }

    private _convert(request: IRequestItem | IChartItem, type: EEntityType) {
        const source = request.request.reg.source;
        this._ng_selectedEntryIndex = -1;
        this._ng_selectedEntry = undefined;
        this._forceUpdate();
        if (type === EEntityType.filters) {
            const err: string | undefined = this._session.getSessionSearch().getChartsAPI().isChartRegExpValid(source);
            if (err !== undefined) {
                return this._notifications.add({
                    caption: 'Charts',
                    message: `Fail to convert into chart, because: ${err}`
                });
            }
            this._session.getSessionSearch().getFiltersAPI().removeStored(source);
            this._session.getSessionSearch().getChartsAPI().addStored(source);
        } else {
            this._session.getSessionSearch().getChartsAPI().removeStored(source);
            this._session.getSessionSearch().getFiltersAPI().addStored(source);
        }
    }

    private _setCurrentFileName(filename: string) {
        this._ng_filename = Toolkit.basename(filename);
        this._filename = filename;
        this._saveState();
        this._cdRef.detectChanges();
    }

    private _onKeyPress(event: KeyboardEvent) {
        if (!this._focused) {
            return;
        }
        switch (event.key) {
            case 'ArrowDown':
                if (this._ng_selectedEntryIndex === -1 || this._ng_selectedEntryIndex === this._ng_entries.length - 1) {
                    this._selectEntryByIndex(0);
                } else {
                    this._selectEntryByIndex(this._ng_selectedEntryIndex + 1);
                }
                break;
            case 'ArrowUp':
                if (this._ng_selectedEntryIndex === -1 || this._ng_selectedEntryIndex === 0) {
                    this._selectEntryByIndex(this._ng_entries.length - 1);
                } else {
                    this._selectEntryByIndex(this._ng_selectedEntryIndex - 1);
                }
                break;
            case 'Enter':
                if (this._ng_selectedEntryIndex !== -1) {
                    const entry = this._ng_entries[this._ng_selectedEntryIndex];
                    if (this._ng_getEntryType(entry) === EEntityType.filters) {
                        this._subjectsRequests.onEdit.next(entry.request as IRequest);
                    } else {
                        this._subjectsCharts.onEdit.next(entry.request as IChartRequest);
                    }
                }
                break;
        }
    }

    private _getStateGuid(): string | undefined {
        if (this._session === undefined) {
            return;
        }
        return `${SidebarAppSearchRequestsComponent.StateKey}:${this._session.getGuid()}`;
    }

    private _loadState(): void {
        if (this._session === undefined) {
            return;
        }
        const key: string | undefined = this._getStateGuid();
        this._session.getSessionsStates().applyStateTo(key, this);
        this._selectEntryByIndex(this._ng_selectedEntryIndex);
        this._forceUpdate();
    }

    private _saveState(): void {
        if (this._session === undefined || this._ng_entries.length === 0) {
            return;
        }
        const key: string | undefined = this._getStateGuid();
        this._session.getSessionsStates().set<IState>(
            key,
            {
                _ng_selectedEntryIndex: this._ng_selectedEntryIndex,
                _ng_filename: this._ng_filename,
                _filename: this._filename,
                _changed: this._changed,
            }
        );
    }

    private _onEntriesUpdated(type: EEntityType | undefined, entries: Array<IRequest | IChartRequest> | undefined) {
        if (type === undefined && entries === undefined) {
            this._ng_entries = [
                ...this._getRequestItems(this._session.getSessionSearch().getFiltersAPI().getStored()),
                ...this._getChartsItems(this._session.getSessionSearch().getChartsAPI().getStored())
            ];
            return this._forceUpdate();
        }
        let prev: IChartRequest | undefined;
        if (this._ng_selectedEntryIndex !== -1 && this._ng_entries[this._ng_selectedEntryIndex] !== undefined) {
            const selected: any = Object.assign({}, this._ng_entries[this._ng_selectedEntryIndex].request);
            if (selected.type !== undefined) {
                prev = selected as IChartRequest;
            }
        } else {
            this._ng_selectedEntry = undefined;
            this._ng_selectedEntryIndex = -1;
        }
        const addedEntry: IRequest | IChartRequest | undefined = this._getNewEntry(entries);
        switch (type) {
            case EEntityType.filters:
                this._ng_entries = [
                    ...this._getRequestItems(entries as IRequest[]),
                    ...this._getChartsItems(this._session.getSessionSearch().getChartsAPI().getStored())
                ];
                break;
            case EEntityType.charts:
                this._ng_entries = [
                    ...this._getRequestItems(this._session.getSessionSearch().getFiltersAPI().getStored()),
                    ...this._getChartsItems(entries as IChartRequest[])
                ];
                break;
        }
        if (this._ng_entries.length === 0 || (this._ng_selectedEntryIndex !== -1 && this._ng_entries[this._ng_selectedEntryIndex] === undefined)) {
            this._ng_selectedEntry = undefined;
            this._ng_selectedEntryIndex = -1;
        } else if (addedEntry !== undefined) {
            this._onSelectEntry(addedEntry);
        } else if (this._ng_selectedEntryIndex !== -1 && prev !== undefined) {
            const selected: any = Object.assign({}, this._ng_entries[this._ng_selectedEntryIndex].request);
            if (selected.type !== undefined && (selected as IChartRequest).type !== prev.type) {
                this._selectEntryByIndex(this._ng_selectedEntryIndex);
            }
        }
        this._forceUpdate();
    }

    private _onSearchProcessing() {
        this._ng_selectedEntry = undefined;
        this._ng_selectedEntryIndex = -1;
        this._forceUpdate();
    }

    private _getRequestItems(requests: IRequest[]): IRequestItem[] {
        return requests.map((request: IRequest) => {
            const _request = Toolkit.copy(request);
            return {
                request: _request,
                onEdit: this._subjectsRequests.onEdit.asObservable(),
                onEditCancel: this._subjectsRequests.onEditCancel.asObservable(),
                onSelect: this._onSelectEntry.bind(this, _request),
                onChangeState: this._onChangeStateRequest.bind(this, _request),
                onEditDone: this._onRequestValueChanged.bind(this, _request),
            };
        });
    }

    private _getChartsItems(charts: IChartRequest[]): IChartItem[] {
        return charts.map((chart: IChartRequest) => {
            const _chart = Toolkit.copy(chart);
            return {
                request: _chart,
                onEdit: this._subjectsCharts.onEdit.asObservable(),
                onEditCancel: this._subjectsCharts.onEditCancel.asObservable(),
                onSelect: this._onSelectEntry.bind(this, _chart),
                onChangeState: this._onChangeStateChart.bind(this, _chart),
                onEditDone: this._onChartValueChanged.bind(this, _chart),
            };
        });
    }

    private _focus() {
        if (this._listRequestsElmRef === undefined) {
            return;
        }
        this._listRequestsElmRef.nativeElement.focus();
    }

    private _getNewEntry(entries: Array<IRequest | IChartRequest>): IRequest | IChartRequest | undefined {
        let result: IRequest | undefined;
        entries.forEach((request: IRequest) => {
            let exist: boolean = false;
            this._ng_entries.forEach((stored: IRequestItem) => {
                if (stored.request.reg.source === request.reg.source) {
                    exist = true;
                }
            });
            if (!exist) {
                result = request;
            }
        });
        this._subjectsRequests.onChanges.next();
        return result;
    }

    private _onRequestValueChanged(request: IRequest, value?: string) {
        this._subjectsRequests.onEditCancel.next();
        this._focus();
        if (value === undefined) {
            return;
        }
        this._session.getSessionSearch().getFiltersAPI().updateStored(request.reg.source, { reguest: value });
        this._subjectsRequests.onChanges.next();
        this._changed = true;
        this._saveState();
    }

    private _onSelectEntry(request: IRequest | IChartRequest) {
        if (this._ng_selectedEntry !== undefined && this._ng_selectedEntry.request.reg.source === request.reg.source) {
            return;
        }
        this._selectEntryByIndex(this._getIndexOfEntry(request));
    }

    private _onRemoveRequest(request: IRequest) {
        this._session.getSessionSearch().getFiltersAPI().removeStored(request.reg.source);
        this._subjectsRequests.onChanges.next();
    }

    private _onChangeStateRequest(request: IRequest, active: boolean) {
        this._session.getSessionSearch().getFiltersAPI().updateStored(request.reg.source, { active: active });
        this._subjectsRequests.onChanges.next();
        this._forceUpdate();
    }

    private _onRemoveChart(request: IChartRequest) {
        this._session.getSessionSearch().getChartsAPI().removeStored(request.reg.source);
        this._subjectsRequests.onChanges.next();
    }

    private _onChangeStateChart(request: IChartRequest, active: boolean) {
        this._session.getSessionSearch().getChartsAPI().updateStored(request.reg.source, { active: active });
        this._subjectsRequests.onChanges.next();
        this._forceUpdate();
    }

    private _onChartValueChanged(request: IChartRequest, value?: string) {
        this._subjectsCharts.onEditCancel.next();
        this._focus();
        if (value === undefined) {
            return;
        }
        this._session.getSessionSearch().getChartsAPI().updateStored(request.reg.source, { reguest: value });
        this._subjectsCharts.onChanges.next();
        this._changed = true;
        this._saveState();
    }

    private _onRequestColorChanged(request: IRequest, color: string, background: string) {
        this._session.getSessionSearch().getFiltersAPI().updateStored(request.reg.source, { color: color, background: background });
        this._subjectsRequests.onChanges.next();
    }

    private _onChartChanged(request: IChartRequest, event: IOnChangeEvent) {
        this._session.getSessionSearch().getChartsAPI().updateStored(request.reg.source, event);
        this._subjectsRequests.onChanges.next();
    }

    private _selectEntryByIndex(index: number) {
        if (this._ng_entries[index] === undefined) {
            return;
        }
        switch (this._ng_getEntryType(this._ng_entries[index])) {
            case EEntityType.filters:
                this._ng_selectedEntry = {
                    request: this._ng_entries[index].request,
                    onChange: this._onRequestColorChanged.bind(this, this._ng_entries[index].request),
                } as IRequestDetailsItem;
                break;
            case EEntityType.charts:
                this._ng_selectedEntry = {
                    request: this._ng_entries[index].request,
                    onChange: this._onChartChanged.bind(this, this._ng_entries[index].request),
                } as IChartDetailsItem;
                break;
        }
        this._ng_selectedEntryIndex = index;
        this._saveState();
        this._forceUpdate();
    }

    private _getIndexOfEntry(request: IRequest | IChartRequest) {
        let index: number = -1;
        this._ng_entries.forEach((item: IRequestItem, i: number) => {
            if (item.request.reg.source === request.reg.source) {
                index = i;
            }
        });
        return index;
    }

    private _onSessionChange(controller: ControllerSessionTab) {
        if (controller === undefined) {
            return;
        }
        this._session = controller;
        this._onEntriesUpdated(undefined, undefined);
    }

    private _toggleAllExcept(source: string | undefined, targetState: boolean) {
        // Set filters
        this._session.getSessionSearch().getFiltersAPI().overwriteStored(this._ng_entries.filter((item: IRequestItem | IChartItem) => {
            return this._ng_getEntryType(item) === EEntityType.filters;
        }).map((item: IRequestItem) => {
            item.request.active = item.request.reg.source === source ? targetState : !targetState;
            return Object.assign({}, item.request);
        }));
        // Set charts
        this._session.getSessionSearch().getChartsAPI().overwriteStored(this._ng_entries.filter((item: IRequestItem | IChartItem) => {
            return this._ng_getEntryType(item) === EEntityType.charts;
        }).map((item: IChartItem) => {
            item.request.active = item.request.reg.source === source ? targetState : !targetState;
            return Object.assign({}, item.request);
        }));
    }

    private _forceUpdate() {
        if (this._destroyed) {
            return;
        }
        this._cdRef.detectChanges();
    }
}
