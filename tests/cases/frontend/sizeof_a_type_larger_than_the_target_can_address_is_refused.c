unsigned int f(void) {
    return sizeof(int[0x3fffffff]) + sizeof(int[0x40000000]);
}
