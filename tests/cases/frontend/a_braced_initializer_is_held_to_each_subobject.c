struct P {
    int *p;
    int n;
};

struct Q {
    int a;
};

union U {
    int i;
    char c;
};

int x;
struct P p;
struct P wrong = {1, 2};
int *two[2] = {1, 2};
int too_many[2] = {1, 2, 3};
struct P nested_too_many = {{&x, 1}};
union U one_member = {1, 2};
struct Q other = {p};
int numbers[2][2] = {"ab"};
int empty[2] = {};
char short_name[2][2] = {"abc"};
char exact[1][2] = {"ab"};
int not_constant[2] = {1, x};
