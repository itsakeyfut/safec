void release(void);
int **get_slot(void);
int f(void) {
    int **pp = get_slot();
    if (pp == 0) {
        return 0;
    }
    int *r = *pp;
    int **pr = &r;
    int *s = *pr;
    if (s == 0) {
        return 0;
    }
    release();
    return *s;
}
