void run(void ***m) {
    if (m == 0) {
        return;
    }
    void **t = *m;
    void ***pt = &t;
    void *pv = pt;
    int ***pi = pv;
    int **a = *pi;
    if (a != 0) {
        int *b = *a;
        if (b != 0) {
            *b = 1;
        }
    }
}
int main(void) {
    char c;
    void *v = &c;
    void **w = &v;
    run(&w);
    return 0;
}
