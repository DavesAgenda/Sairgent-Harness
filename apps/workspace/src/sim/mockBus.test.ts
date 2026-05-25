import { MockBus } from './mockBus';
import type { RuntimeSignal } from '../types';

function makeSignal(type: RuntimeSignal['type'] = 'swo.created'): RuntimeSignal {
  return { type, timestamp: Date.now(), payload: { test: true } };
}

describe('MockBus', () => {
  it('subscribe + emit → callback called with correct signal', () => {
    const bus = new MockBus();
    const received: RuntimeSignal[] = [];
    bus.subscribe((s) => received.push(s));

    const signal = makeSignal('swo.created');
    bus.emit(signal);

    expect(received).toHaveLength(1);
    expect(received[0]).toBe(signal);
  });

  it('multiple subscribers all receive signal', () => {
    const bus = new MockBus();
    const calls: number[] = [];

    bus.subscribe(() => calls.push(1));
    bus.subscribe(() => calls.push(2));
    bus.subscribe(() => calls.push(3));

    bus.emit(makeSignal());

    expect(calls).toContain(1);
    expect(calls).toContain(2);
    expect(calls).toContain(3);
    expect(calls).toHaveLength(3);
  });

  it('unsubscribe → no longer called after unsub', () => {
    const bus = new MockBus();
    const received: RuntimeSignal[] = [];
    const unsub = bus.subscribe((s) => received.push(s));

    bus.emit(makeSignal());
    expect(received).toHaveLength(1);

    unsub();
    bus.emit(makeSignal());
    expect(received).toHaveLength(1);
  });

  it('emit with no subscribers → no error', () => {
    const bus = new MockBus();
    expect(() => bus.emit(makeSignal())).not.toThrow();
  });

  it('unsubscribing one listener does not affect others', () => {
    const bus = new MockBus();
    const receivedA: RuntimeSignal[] = [];
    const receivedB: RuntimeSignal[] = [];

    const unsubA = bus.subscribe((s) => receivedA.push(s));
    bus.subscribe((s) => receivedB.push(s));

    unsubA();
    bus.emit(makeSignal());

    expect(receivedA).toHaveLength(0);
    expect(receivedB).toHaveLength(1);
  });
});
