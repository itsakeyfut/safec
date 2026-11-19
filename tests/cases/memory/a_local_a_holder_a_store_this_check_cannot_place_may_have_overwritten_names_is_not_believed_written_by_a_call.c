void stash(int ***pp, int ****box); void fill(int **p);
int main(void) {
    int *a;
    int *b = 0;
    int **h = &b;
    int ***cell = 0;
    stash(&h, &cell);
    int ***t = cell;
    h = &a;
    if (t) *t = &b;
    fill(h);
    return a != 0;
}
