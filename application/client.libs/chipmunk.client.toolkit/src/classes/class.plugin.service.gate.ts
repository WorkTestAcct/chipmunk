import { APluginService } from "./class.plugin.service";
import { ARowCommonParser } from './class.parser.row.common';
import { ARowTypedParser } from './class.parser.row.typed';
import { ARowBoundParser } from './class.parser.row.bound';
import { ATypedRowRender } from './class.typedrow.render';
import { ASelectionParser } from './class.parser.selection';

export enum ECoreModules {
    '@angular/core' = '@angular/core',
    '@angular/common' = '@angular/common',
    '@angular/forms' = '@angular/forms',
    '@angular/platform-browser' = '@angular/platform-browser',
    'rxjs' = 'rxjs',
    'chipmunk-client-complex' = 'chipmunk-client-complex',
    'chipmunk-client-containers' = 'chipmunk-client-containers',
    'chipmunk-client-primitive' = 'chipmunk-client-primitive',
    'chipmunk.client.toolkit' = 'chipmunk.client.toolkit',
    'xterm' = 'xterm',
    'xterm/lib/addons/fit/fit' = 'xterm/lib/addons/fit/fit',
    'electron' = 'electron',
}

export interface ICoreModules {
    '@angular/core': any;
    '@angular/common': any;
    '@angular/forms': any;
    '@angular/platform-browser': any;
    'rxjs': any;
    'chipmunk-client-complex': any;
    'chipmunk-client-containers': any;
    'chipmunk-client-primitive': any;
    'chipmunk.client.toolkit': any;
    'xterm': any;
    'xterm/lib/addons/fit/fit': any;
    'electron': any;
}

export type TPluginExportEntity = ARowCommonParser | ARowTypedParser | ARowBoundParser | ATypedRowRender<any> | ASelectionParser | APluginService;

export type TRequire = (module: ECoreModules) => any;

export interface IPluginExports {
    [key: string]: TPluginExportEntity;
}

/**
 * Used for none-Angular plugins to delivery plugin's exports into the core of chipmunk
 * Developer can create none-Angular plugin. In global namespace of the main javascript file will be
 * available implementation of PluginServiceGate.
 * For example:
 * =================================================================================================
 * const gate: Toolkit.APluginServiceGate | undefined = (window as any).logviewer;
 * gate.setPluginExports({
 *     parser: new MyParserOfEachRow(),
 * });
 * =================================================================================================
 * This code snippet registered a new parser for output "MyParserOfEachRow"
 * @usecases should be used for none-angular plugins to register parsers
 * @class APluginServiceGate
 */
export abstract class APluginServiceGate {
    /**
     * Internal usage
     */
    public abstract setPluginExports(exports: IPluginExports): void;

    /**
     * Internal usage
     */
    public abstract getCoreModules(): ICoreModules;

    /**
     * Internal usage
     */
    public abstract getRequireFunc(): TRequire;

}
