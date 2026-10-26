void *malloc(int n);
void free(void *p);
void run(void **m) {
    void ***box = malloc(8);
    if (box == 0) {
        return;
    }
    *box = m;
    void *pb = box;
    int ***pi = pb;
    if (*pi != 0) {
        int *b = **pi;
        if (b != 0) {
            *b = 1;
        }
    }
    free(box);
}
