import * as Components from './components';
import * as Enums from '../consts/enums';
import { ControllerViewportEvents } from '../controllers/controller.viewport.events';
import { ControllerSessionsEvents } from '../controllers/controller.sessions.events';
import { PluginIPC } from '../classes/class.ipc';
import { IPopup } from './client.popup';
import { IComponentDesc } from './client.components.containers';
import { INotification } from './client.notification';

/**
 * Plugin's API. Gives access to:
 * - major core events
 * - core's UI
 * - plugin IPC (to communicate between host and render of plugin)
 */
export interface IAPI {
    /**
     * @returns {PluginIPC} Returns PluginAPI object for host and render plugin communication
     */
    getIPC: () => PluginIPC | undefined;

    /**
     * @returns {string} ID of active stream (active tab)
     */
    getActiveSessionId: () => string;

    /**
     * Allows adding injection into the main view. Injection should be an Angular component.
     * @param {IComponentInjection} injection - Angular compenent
     * @param {EViewsTypes} type - type of injection: location (main view, search view, sidebar) and position (top, bottom)
     * @returns {void}
     */
    addOutputInjection: (injection: Components.IComponentInjection, type: Enums.EViewsTypes) => void;

    /**
     * Used for removing injection, defined with addOutputInjection
     * @param {string} id - id of injection (property of IComponentInjection)
     * @param {EViewsTypes} type - type of injection
     * @returns {void}
     */
    removeOutputInjection: (id: string, type: Enums.EViewsTypes) => void;

    /**
     * Returns hub of viewport events (resize, update and so on)
     * Should be used to track state of viewport
     * @returns {ControllerViewportEvents} viewport events hub
     */
    getViewportEventsHub: () => ControllerViewportEvents;

    /**
     * Returns hub of sessions events (open, close, changed and so on)
     * Should be used to track active sessions
     * @returns {ControllerSessionsEvents} sessions events hub
     */
    getSessionsEventsHub: () => ControllerSessionsEvents;

    /**
     * Open popup
     * @param {IPopup} popup - description of popup
     */
    addPopup: (popup: IPopup) => string;

    /**
     * Closes popup
     * @param {string} guid - id of existing popup
     */
    removePopup: (guid: string) => void;

    /**
     * Adds sidebar title injection.
     * This method doesn't need "delete" method, because sidebar injection would be
     * removed with a component, which used as sidebar tab render.
     * In any way developer could define an argument as "undefined" to force removing
     * injection from the title of sidebar
     * @param {IComponentDesc} component - description of Angular component
     * @returns {void}
     */
    setSidebarTitleInjection: (component: IComponentDesc | undefined) => void;

    /**
     * Opens sidebar app by ID
     * @param {string} appId - id of app
     * @param {boolean} silence - do not make tab active
     */
    openSidebarApp: (appId: string, silence: boolean) => void;

    /**
     * Opens toolbar app by ID
     * @param {string} appId - id of app
     * @param {boolean} silence - do not make tab active
     */
    openToolbarApp: (appId: string, silence: boolean) => void;

    /**
     * Adds new notification
     * @param {INotification} notification - notification to be added
     */
    addNotification: (notification: INotification) => void;
}
