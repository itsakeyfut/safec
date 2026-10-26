void put(void **slot, char *c) {
    if (slot != 0) {
        *slot = c;
    }
}

int main(void) {
    char c;
    void *v = 0;
    put(&v, &c);
    void *pv = &v;
    int **pi = pv;
    int *p = *pi;
    if (p != 0) {
        *p = 1;
    }
    return 0;
}
