int strcmp(const char *, const char *);

struct S {
    int x, y;
};

const char *names[] = {"a", "bc"};
const char *later = "ab" + 1;
int g = 9;
int *p = &g;
int a[4] = {1, 2, 3, 4};
int *third = &a[2];
int *last = a + 3;
struct S s = {5, 6};
int *y = &s.y;
extern int h;
int *ahead = &h;
int h = 11;

struct Entry {
    const char *name;
    int *count;
} table[2] = {{"first", &g}, {"second", &s.x}};

int main(void) {
    if (strcmp(names[0], "a") != 0 || strcmp(names[1], "bc") != 0 || *later != 'b') {
        return 1;
    }
    if (*p != 9 || *third != 3 || *last != 4 || *y != 6 || *ahead != 11) {
        return 2;
    }
    if (strcmp(table[1].name, "second") != 0 || *table[0].count != 9 || *table[1].count != 5) {
        return 3;
    }
    return 0;
}
