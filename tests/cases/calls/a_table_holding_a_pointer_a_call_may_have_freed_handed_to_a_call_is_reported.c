void *malloc(int n);
void release(int *p);
int use2(int **pp);

int main(void) {
    int **t = malloc(8);
    if (t == 0) {
        return 0;
    }
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    *t = a;
    release(a);
    return use2(t);
}
