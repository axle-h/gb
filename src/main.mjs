import { readFileSync } from 'node:fs';


const opcodes = JSON.parse(readFileSync('./opcodes.json'));
Object.entries(opcodes['unprefixed']).map(([opcode, { mnemonic, operands, cycles }]) => {

    let testNames = [mnemonic];
    const machineCode = [opcode];
    const operandString = operands
        .map(({ name, immediate, increment, decrement }) => {
            let value = name;
            if (increment) {
                value += '+';
                testNames.push(`${name}_increment`);
            } else if (decrement) {
                value += '-';
                testNames.push(`${name}_decrement`);
            } else {
                testNames.push(name);
            }
            switch (name) {
                case 'n16':
                case 'a16':
                    machineCode.push('0x34');
                    machineCode.push('0x12');
                    value = '0x1234';
                    break;
                case 'n8':
                case 'a8':
                    machineCode.push('0x12');
                    value = '0x12';
                    break;
                case 'e8':
                    machineCode.push('0x7B');
                    value = '123';
                    break;
            }

            return immediate ? value : `(${value})`
        })
        .join(', ');

    const assembly = `${mnemonic} ${operandString}`.trim();
    const testName = testNames.join('_').toLowerCase().replaceAll('$', '')
    const machine_cycles = cycles[0] / 4;
    console.log(`            ${testName}: ${machineCode.join(", ")} => "${assembly}", ${machine_cycles},`);
})