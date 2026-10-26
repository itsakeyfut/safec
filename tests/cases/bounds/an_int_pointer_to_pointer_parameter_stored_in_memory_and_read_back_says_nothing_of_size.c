void *malloc(int n);
void free(void *p);
void run(int **m) {
    int ***box = malloc(8);
    if (box == 0) {
        return;
    }
    *box = m;
    int **a = *box;
    if (a != 0) {
        int *b = *a;
        if (b != 0) {
            *b = 1;
        }
    }
    free(box);
}
