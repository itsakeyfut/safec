void *malloc(int n);
void free(void *p);
void drop_all(int ***hh);

int main(void) {
    int ***hh = malloc(8);
    if (hh == 0) {
        return 0;
    }
    int **box = malloc(8);
    if (box == 0) {
        return 0;
    }
    *hh = box;
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    **hh = p;
    drop_all(hh);
    return *p;
}
