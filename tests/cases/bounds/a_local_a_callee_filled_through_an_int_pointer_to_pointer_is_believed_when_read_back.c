void fill(int **out);

int main(void) {
    int *v = 0;
    fill(&v);
    int **pi = &v;
    int *q = *pi;
    if (q != 0) {
        *q = 1;
    }
    return 0;
}
