void *malloc(int n);
void free(void *p);
void run(void ***m) {
    void ***box = malloc(8);
    if (box == 0) {
        return;
    }
    if (m == 0) {
        free(box);
        return;
    }
    void **t = *m;
    *box = t;
    void *pb = box;
    int ***pi = pb;
    int **a = *pi;
    if (a != 0) {
        int *b = *a;
        if (b != 0) {
            *b = 1;
        }
    }
    free(box);
}
