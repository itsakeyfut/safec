void *malloc(int n);
void free(void *p);
void *memcpy(void *s1, void *s2, int n);
void run(void **m) {
    void ***src = malloc(8);
    if (src == 0) { return; }
    void ***box = malloc(8);
    if (box == 0) { free(src); return; }
    *src = m;
    memcpy(box, src, 8);
    void *pb = box;
    int ***pi = pb;
    int **a = *pi;
    if (a != 0) {
        int *b = *a;
        if (b != 0) {
            *b = 1;
        }
    }
    free(src);
    free(box);
}
