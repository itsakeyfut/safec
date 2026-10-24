void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(void) {
    int **box = malloc(8);
    if (box == 0) { return 1; }
    int **nb = realloc(box, 16);
    if (nb == 0) {
        int *t = *box;
        if (t != 0) { *t = 1; }
        free(box);
        return 0;
    }
    free(nb);
    return 0;
}
