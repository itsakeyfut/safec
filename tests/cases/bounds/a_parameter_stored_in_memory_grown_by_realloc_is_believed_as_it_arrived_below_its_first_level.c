void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
void run(void **m) {
    void ***box = malloc(8);
    if (box == 0) {
        return;
    }
    *box = m;
    void *grown = realloc(box, 16);
    if (grown == 0) {
        free(box);
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
