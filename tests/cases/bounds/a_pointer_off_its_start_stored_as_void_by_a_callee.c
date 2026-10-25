void put(void **slot, int *p) {
    if (slot != 0) {
        if (p != 0) {
            *slot = p + 5;
        }
    }
}
