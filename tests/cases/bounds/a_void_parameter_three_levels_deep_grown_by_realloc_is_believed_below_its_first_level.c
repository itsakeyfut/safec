void *realloc(void *p, int n);
void free(void *p);
void run(void ***m) {
    void *pm = m;
    void *grown = realloc(pm, 16);
    if (grown == 0) {
        return;
    }
    int ***pi = grown;
    int **a = *pi;
    if (a != 0) {
        int *b = *a;
        if (b != 0) {
            *b = 1;
        }
    }
    free(grown);
}
