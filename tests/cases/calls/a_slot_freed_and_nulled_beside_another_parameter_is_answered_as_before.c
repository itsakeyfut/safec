void free(void *p);
void release(int **slot, int **other) {
    if (!slot) return;
    free(*slot);
    *slot = 0;
    if (!other) return;
    int *o = *other;
    if (!o) return;
    *o = 1;
}
