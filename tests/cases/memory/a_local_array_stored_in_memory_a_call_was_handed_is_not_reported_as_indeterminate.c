void *malloc(int n);
void fill_box(int ***box);

int main(void) {
    int *m[1];
    int ***box = malloc(8);
    if (box == 0) {
        return 1;
    }
    *box = m;
    fill_box(box);
    int *q = m[0];
    return q == 0;
}
