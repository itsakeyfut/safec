void *malloc(int n);
void free(void *p);
void put(int *p);
int f(int *q) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    put(*box);
    free(box);
    return 0;
}
