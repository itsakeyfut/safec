void release_all(void);

int f(int *p) {
    if (p == 0) {
        return 0;
    }
    release_all();
    return *p;
}
