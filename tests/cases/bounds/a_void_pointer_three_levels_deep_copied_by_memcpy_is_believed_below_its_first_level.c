void *malloc(int n);
void free(void *p);
void *memcpy(void *s1, void *s2, int n);
void run(void ***m) {
    void ***box = malloc(8);
    if (box == 0) {
        return;
    }
    memcpy(box, m, 8);
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
