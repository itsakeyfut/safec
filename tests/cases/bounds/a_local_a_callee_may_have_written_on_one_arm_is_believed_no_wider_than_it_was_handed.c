void put(void **slot, char *c) {
    if (slot != 0) {
        *slot = c;
    }
}

int main(int argc) {
    char c;
    int *v = 0;
    int **pi = &v;
    void *pv = pi;
    if (argc) {
        put(pv, &c);
    }
    int *q = *pi;
    if (q != 0) {
        *q = 1;
    }
    return 0;
}
