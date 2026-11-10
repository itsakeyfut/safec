void *malloc(int n);
void stored(int ***** _Nonnull out) {
    int **c = malloc(8);
    int ***b = malloc(8);
    if (!b) return;
    *b = c;
    int ****box = malloc(8);
    if (!box) return;
    *box = b;
    *out = box;
}
