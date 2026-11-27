int strcmp(const char *, const char *);

struct S {
    int a, b;
};

int *gp = (int[]){3, 4};
struct S *gs = &(struct S){5, 6};
const char *const *names = (const char *[]){"x", "yz"};

static int sum(const int *p, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += p[i];
    }
    return s;
}

int main(void) {
    struct S s = (struct S){1, 2};
    int *p = (int[]){7, 8, 9};
    int total = 0;
    for (int i = 0; i < 3; i++) {
        int *q = (int[]){i, i * 10};
        q[0]++;
        total += q[0] + q[1];
    }
    int n = (int){42};
    int z = ((struct S){.b = 3}).b;
    int k = (int[]){5, 6}[1];
    if (s.a != 1 || s.b != 2 || p[2] != 9 || total != 36 || n != 42 || z != 3 || k != 6) {
        return 1;
    }
    if (gp[1] != 4 || gs->b != 6 || strcmp(names[1], "yz") != 0) {
        return 2;
    }
    if (sum((int[]){1, 2, 3}, 3) != 6 || sizeof((int[]){1, 2, 3}) != 12) {
        return 3;
    }
    return 0;
}
