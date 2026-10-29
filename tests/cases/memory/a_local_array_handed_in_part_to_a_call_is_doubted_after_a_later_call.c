void fill(int **s);
void tick(void);
int main(void) {
    int *m[2];
    fill(m + 1);
    tick();
    int *r = m[0];
    return r != 0;
}
