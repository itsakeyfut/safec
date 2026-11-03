void put(void **slot, char *c) {
    if (slot != 0) {
        *slot = c;
    }
}

int main(int argc) {
    char c;
    int x = 0;
    int *v = &x;
    int **pi = &v;
    void *pv = pi;
    if (argc) {
        put(pv, &c);
    }
    **pi = 1;
    return x;
}
