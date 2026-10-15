import calendar
import datetime
import json
import sys


def refuse(why):
    print(why, file=sys.stderr)
    sys.exit(2)


def day(value, name):
    try:
        return datetime.date.fromisoformat(value)
    except (TypeError, ValueError):
        refuse(f"{name} must be a date written YYYY-MM-DD, not {value!r}")


def add_months(start, months):
    month = start.month - 1 + months
    year = start.year + month // 12
    month = month % 12 + 1
    return start.replace(year=year, month=month, day=min(start.day, calendar.monthrange(year, month)[1]))


def add_business_days(start, days):
    step = 1 if days >= 0 else -1
    current, left = start, abs(days)
    while left:
        current += datetime.timedelta(days=step)
        if current.weekday() < 5:
            left -= 1
    return current


def business_days_between(start, end):
    if end < start:
        return -business_days_between(end, start)
    return sum(1 for n in range((end - start).days) if (start + datetime.timedelta(days=n + 1)).weekday() < 5)


def described(date):
    return {"date": date.isoformat(), "weekday": date.strftime("%A")}


args = json.load(sys.stdin)
if not isinstance(args, dict):
    refuse("the arguments must be an object")
start = day(args.get("start"), "start")
end = args.get("end")
days = args.get("add_days", 0)
months = args.get("add_months", 0)
business = args.get("business_days", False)
if not isinstance(days, int) or not isinstance(months, int) or not isinstance(business, bool):
    refuse("add_days and add_months are whole numbers and business_days is true or false")
if end is None and not days and not months:
    refuse("give an end date to count to, or add_days or add_months to move by")

result = {"start": described(start)}
if end is not None:
    end = day(end, "end")
    result["end"] = described(end)
    result["days_between"] = (end - start).days
    result["weeks_between"] = round((end - start).days / 7, 2)
    result["business_days_between"] = business_days_between(start, end)
if days or months:
    moved = add_months(start, months)
    moved = add_business_days(moved, days) if business else moved + datetime.timedelta(days=days)
    result["result"] = described(moved)
print(json.dumps(result))
